use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use reqwest::{Client, Request, RequestBuilder};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use worklens_core::raw::Call;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{status} for {url}\n{body}")]
    Status { status: u16, url: String, body: String },
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error("{what}: unexpected response: {source}")]
    Parse { what: &'static str, source: serde_json::Error },
    #[error("{0}")]
    Api(String),
    #[error("rate limited, try again after {}", clock(*until))]
    RateLimited { until: Timestamp },
}

impl Error {
    pub fn summary(&self) -> String {
        self.to_string().lines().next().unwrap_or_default().to_owned()
    }

    pub const fn retry_at(&self) -> Option<Timestamp> {
        match self {
            Self::RateLimited { until } => Some(*until),
            _ => None,
        }
    }
}

fn clock(t: Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string()
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

const PER_HOST: usize = 2;

use tokio::sync::Semaphore;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    Bugzilla,
    Phabricator,
    Jira,
    Treeherder,
    ProductDetails,
    Probeinfo,
    CrashStats,
    Other,
}

impl Service {
    const COUNT: usize = 8;

    fn of(host: &str) -> Self {
        match host {
            "bugzilla.mozilla.org" => Self::Bugzilla,
            "phabricator.services.mozilla.com" => Self::Phabricator,
            "api.atlassian.com" => Self::Jira,
            h if h.ends_with(".atlassian.net") => Self::Jira,
            "treeherder.mozilla.org" => Self::Treeherder,
            "product-details.mozilla.org" => Self::ProductDetails,
            "probeinfo.telemetry.mozilla.org" => Self::Probeinfo,
            "crash-stats.mozilla.org" => Self::CrashStats,
            _ => Self::Other,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

pub fn parse<T: DeserializeOwned>(what: &'static str, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|source| Error::Parse { what, source })
}

enum Clock {
    Real,
    Virtual(AtomicI64),
}

impl Clock {
    fn now(&self) -> Timestamp {
        match self {
            Self::Real => Timestamp::now(),
            Self::Virtual(ms) => Timestamp::from_millisecond(ms.load(Ordering::SeqCst)).unwrap_or(Timestamp::UNIX_EPOCH),
        }
    }

    async fn sleep(&self, d: Duration) {
        match self {
            Self::Real => tokio::time::sleep(d).await,
            Self::Virtual(ms) => {
                let target = ms
                    .load(Ordering::SeqCst)
                    .saturating_add(i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
                tokio::task::yield_now().await;
                ms.fetch_max(target, Ordering::SeqCst);
            }
        }
    }
}

pub struct Http {
    pub client: Client,
    calls: CallLog,
    replay: Option<Replay>,
    clock: Clock,
    cooldowns: [AtomicI64; Service::COUNT],
    permits: [Semaphore; Service::COUNT],
    seed: AtomicU64,
    wait_ms: AtomicU64,
}

pub struct Replay {
    dir: std::path::PathBuf,
    routes: Vec<Route>,
}

struct Route {
    method: String,
    url: reqwest::Url,
    file: String,
    answer: Answer,
    used: AtomicU32,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    status: Option<u16>,
    retry_after: Option<String>,
    times: Option<u32>,
    body: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RouteSpec {
    Plain(String, String),
    With(String, String, Answer),
}

struct Reply {
    status: u16,
    retry_after: Option<String>,
    body: Vec<u8>,
}

impl Replay {
    pub fn load(dir: &std::path::Path) -> Result<Self> {
        let path = dir.join("routes.json");
        let bytes = std::fs::read(&path).map_err(|e| Error::Api(format!("replay: {}: {e}", path.display())))?;
        let specs: Vec<RouteSpec> = parse("replay routes", &bytes)?;
        let routes = specs
            .into_iter()
            .map(|spec| {
                let (spec, file, answer) = match spec {
                    RouteSpec::Plain(spec, file) => (spec, file, Answer::default()),
                    RouteSpec::With(spec, file, answer) => (spec, file, answer),
                };
                let (method, url) = spec.split_once(' ').unwrap_or(("GET", spec.as_str()));
                let url = reqwest::Url::parse(url).map_err(|e| Error::Api(format!("replay: route {spec:?}: {e}")))?;
                Ok(Route {
                    method: method.to_owned(),
                    url,
                    file,
                    answer,
                    used: AtomicU32::new(0),
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            dir: dir.to_owned(),
            routes,
        })
    }

    fn answer(&self, method: &str, url: &reqwest::Url, body: &str) -> Result<Reply> {
        let route = self
            .routes
            .iter()
            .find(|r| {
                r.matches(method, url)
                    && r.answer.body.as_deref().is_none_or(|want| body.contains(want))
                    && r.answer.times.is_none_or(|n| r.used.load(Ordering::SeqCst) < n)
            })
            .ok_or_else(|| Error::Api(format!("replay: nothing recorded for {method} {url}")))?;
        route.used.fetch_add(1, Ordering::SeqCst);
        let path = self.dir.join(&route.file);
        let body = std::fs::read(&path).map_err(|e| Error::Api(format!("replay: {}: {e}", path.display())))?;
        Ok(Reply {
            status: route.answer.status.unwrap_or(200),
            retry_after: route.answer.retry_after.clone(),
            body,
        })
    }
}

impl Route {
    fn matches(&self, method: &str, url: &reqwest::Url) -> bool {
        let segments = |u: &reqwest::Url| u.path().split('/').map(str::to_owned).collect::<Vec<_>>();
        let (want, got) = (segments(&self.url), segments(url));
        let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        self.method == method
            && self.url.host_str() == url.host_str()
            && want.len() == got.len()
            && want.iter().zip(&got).all(|(w, g)| w == "*" || w == g)
            && self
                .url
                .query_pairs()
                .all(|(k, v)| pairs.iter().any(|(pk, pv)| *pk == k && *pv == v))
    }
}

fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

impl Http {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent("worklens (read-only)")
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        let replay = std::env::var_os("WORKLENS_REPLAY")
            .map(|dir| Replay::load(std::path::Path::new(&dir)))
            .transpose()?;
        Ok(Self::with_replay(client, replay))
    }

    fn with_replay(client: Client, replay: Option<Replay>) -> Self {
        let now = Timestamp::now();
        let (clock, seed) = if replay.is_some() {
            (Clock::Virtual(AtomicI64::new(now.as_millisecond())), 0x9e37_79b9_7f4a_7c15)
        } else {
            (Clock::Real, now.as_nanosecond().unsigned_abs() as u64 | 1)
        };
        Self {
            client,
            calls: CallLog::default(),
            replay,
            clock,
            cooldowns: Default::default(),
            permits: std::array::from_fn(|_| Semaphore::new(PER_HOST)),
            seed: AtomicU64::new(seed),
            wait_ms: AtomicU64::new(0),
        }
    }

    pub(crate) fn restore_cooldowns(&self, raw: &worklens_core::Raw) {
        for error in raw.failures() {
            let Some(until) = error.retry_at else { continue };
            let service = match error.source.split('-').next().unwrap_or_default() {
                "bugzilla" => Service::Bugzilla,
                "phabricator" => Service::Phabricator,
                "jira" => Service::Jira,
                "treeherder" => Service::Treeherder,
                "product" => Service::ProductDetails,
                "probeinfo" => Service::Probeinfo,
                "crash" => Service::CrashStats,
                _ => continue,
            };
            self.cooldowns[service.index()].fetch_max(until.as_millisecond(), Ordering::SeqCst);
        }
    }

    fn jitter(&self) -> f64 {
        let mut x = self.seed.load(Ordering::Relaxed);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.seed.store(x, Ordering::Relaxed);
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    async fn wait(&self, delay: Duration, service: Service) -> Result<()> {
        let ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        if self
            .wait_ms
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                used.checked_add(ms).filter(|total| *total <= 30_000)
            })
            .is_err()
        {
            let until = self.cooldowns[service.index()].load(Ordering::SeqCst);
            return Err(if until > self.clock.now().as_millisecond() {
                Error::RateLimited {
                    until: Timestamp::from_millisecond(until).unwrap_or(Timestamp::MAX),
                }
            } else {
                Error::Api("Refresh retry wait budget exhausted; retry refresh later".into())
            });
        }
        self.clock.sleep(delay).await;
        Ok(())
    }

    async fn cool_down(&self, service: Service) -> Result<Duration> {
        let mut waited = Duration::ZERO;
        loop {
            let until = self.cooldowns[service.index()].load(Ordering::SeqCst);
            let now = self.clock.now().as_millisecond();
            if until <= now {
                return Ok(waited);
            }
            let left = Duration::from_millis(u64::try_from(until - now).unwrap_or(0));
            if left > MAX_WAIT {
                return Err(Error::RateLimited {
                    until: Timestamp::from_millisecond(until).unwrap_or(Timestamp::MAX),
                });
            }
            self.wait(left, service).await?;
            waited += left;
        }
    }

    pub async fn send(&self, req: RequestBuilder) -> Result<Vec<u8>> {
        let mut req = req.build()?;
        let service = Service::of(req.url().host_str().unwrap_or_default());
        let mut attempt = 1;
        let mut waited = Duration::ZERO;
        loop {
            waited += self.cool_down(service).await?;
            let copy = req.try_clone();
            let (result, retry) = self.attempt(req, attempt, service, waited).await;
            match (retry, copy) {
                (Retry::After(delay), Some(copy)) => {
                    self.wait(delay, service).await?;
                    waited = delay;
                    req = copy;
                    attempt += 1;
                }
                (Retry::Limited(until), _) => return Err(Error::RateLimited { until }),
                _ => {
                    let until = self.cooldowns[service.index()].load(Ordering::SeqCst);
                    if result.is_err() && until > self.clock.now().as_millisecond() {
                        return Err(Error::RateLimited {
                            until: Timestamp::from_millisecond(until).unwrap_or(Timestamp::MAX),
                        });
                    }
                    return result;
                }
            }
        }
    }

    async fn attempt(&self, req: Request, attempt: u32, service: Service, waited: Duration) -> (Result<Vec<u8>>, Retry) {
        let _permit = self.permits[service.index()]
            .acquire()
            .await
            .expect("HTTP semaphores are never closed");
        let waited = match self.cool_down(service).await {
            Ok(extra) => waited + extra,
            Err(error) => return (Err(error), Retry::Stop),
        };
        let (method, url) = (req.method().to_string(), req.url().to_string());
        let (at, started) = (self.clock.now(), Instant::now());
        let record = |status: Option<u16>, error: Option<String>| {
            let ms = if self.replay.is_some() { 0 } else { millis(started.elapsed()) };
            self.calls.push(Call {
                at,
                method: method.clone(),
                url: url.clone(),
                status,
                ms,
                error,
                wait_ms: millis(waited),
            });
        };
        let reply = if let Some(replay) = &self.replay {
            let body = req.body().and_then(|b| b.as_bytes()).unwrap_or_default();
            let body = reqwest::Url::parse(&format!("http://form/?{}", String::from_utf8_lossy(body)))
                .map(|u| u.query_pairs().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"))
                .unwrap_or_default();
            match replay.answer(&method, req.url(), &body) {
                Ok(reply) => reply,
                Err(e) => {
                    record(None, Some(e.to_string()));
                    return (Err(e), Retry::Stop);
                }
            }
        } else {
            let res = match self.client.execute(req).await {
                Ok(res) => res,
                Err(e) => {
                    record(None, Some(e.to_string()));
                    let retry = if e.is_connect() || e.is_timeout() {
                        retry_delay(None, None, attempt, at, self.jitter())
                    } else {
                        Retry::Stop
                    };
                    return (Err(e.into()), retry);
                }
            };
            let status = res.status().as_u16();
            let retry_after = res
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            match res.bytes().await {
                Ok(body) => Reply {
                    status,
                    retry_after,
                    body: body.to_vec(),
                },
                Err(e) => {
                    record(Some(status), Some(e.to_string()));
                    return (Err(e.into()), Retry::Stop);
                }
            }
        };
        record(Some(reply.status), None);
        if (200..300).contains(&reply.status) {
            return (Ok(reply.body), Retry::Stop);
        }
        let now = self.clock.now();
        if let Some(until) = cooldown_until(reply.status, reply.retry_after.as_deref(), now) {
            self.cooldowns[service.index()].fetch_max(until.as_millisecond(), Ordering::SeqCst);
        }
        let text = String::from_utf8_lossy(&reply.body);
        let error = Error::Status {
            status: reply.status,
            url: url.split('?').next().unwrap_or_default().to_owned(),
            body: text.chars().take(300).collect(),
        };
        let retry = retry_delay(Some(reply.status), reply.retry_after.as_deref(), attempt, now, self.jitter());
        (Err(error), retry)
    }

    pub fn take_calls(&self) -> Vec<Call> {
        self.calls.take()
    }
}

const MAX_ATTEMPTS: u32 = 3;
const MAX_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retry {
    Stop,
    After(Duration),
    Limited(Timestamp),
}

fn asked_wait(value: &str, now: Timestamp) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = jiff::fmt::rfc2822::DateTimeParser::new().parse_timestamp(value).ok()?;
    let ms = at.as_millisecond().saturating_sub(now.as_millisecond()).max(0);
    Some(Duration::from_millis(u64::try_from(ms).unwrap_or(0)))
}

fn backoff(attempt: u32, jitter: f64) -> Duration {
    let ceiling = 2f64.powi(i32::try_from(attempt.saturating_sub(1)).unwrap_or(0));
    Duration::from_secs_f64(ceiling * jitter.clamp(0.0, 1.0))
}

const fn retryable(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

fn cooldown_until(status: u16, retry_after: Option<&str>, now: Timestamp) -> Option<Timestamp> {
    if !matches!(status, 429 | 503) {
        return None;
    }
    let wait = asked_wait(retry_after?, now)?;
    now.checked_add(jiff::SignedDuration::try_from(wait).ok()?).ok()
}

fn retry_delay(status: Option<u16>, retry_after: Option<&str>, attempt: u32, now: Timestamp, jitter: f64) -> Retry {
    let asked = retry_after.and_then(|v| asked_wait(v, now));
    if let (Some(status), Some(wait)) = (status, asked)
        && retryable(status)
        && wait > MAX_WAIT
    {
        let until = jiff::SignedDuration::try_from(wait)
            .ok()
            .and_then(|d| now.checked_add(d).ok())
            .unwrap_or(Timestamp::MAX);
        return Retry::Limited(until);
    }
    if attempt >= MAX_ATTEMPTS {
        return Retry::Stop;
    }
    match status {
        None => Retry::After(backoff(attempt, jitter)),
        Some(s) if retryable(s) => Retry::After(asked.unwrap_or_else(|| backoff(attempt, jitter))),
        Some(_) => Retry::Stop,
    }
}

#[derive(Default)]
struct CallLog(
    #[expect(
        clippy::disallowed_types,
        reason = "append-only log, locked only to push or take and never across an await"
    )]
    std::sync::Mutex<Vec<Call>>,
);

impl CallLog {
    fn push(&self, call: Call) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(call);
    }

    fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn now() -> Timestamp {
        at("2015-10-21T07:27:50Z")
    }

    fn after(secs: f64) -> Retry {
        Retry::After(Duration::from_secs_f64(secs))
    }

    #[test]
    fn waits_the_seconds_or_until_the_date_the_server_asks() {
        assert_eq!(retry_delay(Some(429), Some("3"), 1, now(), 0.5), after(3.0));
        assert_eq!(retry_delay(Some(429), Some(" 25 "), 1, now(), 0.5), after(25.0));
        assert_eq!(
            retry_delay(Some(503), Some("Wed, 21 Oct 2015 07:28:00 GMT"), 1, now(), 0.5),
            after(10.0)
        );
    }

    #[test]
    fn a_past_date_or_zero_means_retry_now() {
        assert_eq!(retry_delay(Some(504), Some("0"), 1, now(), 0.5), after(0.0));
        assert_eq!(
            retry_delay(Some(429), Some("Wed, 21 Oct 2015 07:00:00 GMT"), 1, now(), 0.5),
            after(0.0)
        );
    }

    #[test]
    fn garbage_or_no_hint_backs_off_exponentially_with_jitter() {
        assert_eq!(retry_delay(Some(503), Some("soon"), 1, now(), 0.5), after(0.5));
        assert_eq!(retry_delay(Some(429), Some("-3"), 2, now(), 0.5), after(1.0));
        assert_eq!(retry_delay(Some(502), None, 1, now(), 1.0), after(1.0));
        assert_eq!(retry_delay(Some(502), None, 2, now(), 1.0), after(2.0));
        assert_eq!(retry_delay(None, None, 2, now(), 0.25), after(0.5));
        assert_eq!(retry_delay(None, None, 1, now(), 0.0), after(0.0));
    }

    #[test]
    fn a_delay_past_the_budget_is_a_rate_limit_with_its_retry_time() {
        assert_eq!(
            retry_delay(Some(429), Some("120"), 1, now(), 0.5),
            Retry::Limited(at("2015-10-21T07:29:50Z"))
        );
        assert_eq!(
            retry_delay(Some(503), Some("Wed, 21 Oct 2015 08:00:00 GMT"), 3, now(), 0.5),
            Retry::Limited(at("2015-10-21T08:00:00Z")),
            "even out of attempts, the error says when to try again"
        );
        assert_eq!(retry_delay(Some(429), Some("30"), 1, now(), 0.5), after(30.0));
        assert_eq!(
            retry_delay(Some(429), Some("31"), 1, now(), 0.5),
            Retry::Limited(at("2015-10-21T07:28:21Z"))
        );
    }

    #[test]
    fn attempts_stay_bounded() {
        assert_eq!(retry_delay(Some(429), Some("3"), 3, now(), 0.5), Retry::Stop);
        assert_eq!(retry_delay(None, None, 3, now(), 0.5), Retry::Stop);
        assert_eq!(retry_delay(Some(503), None, 2, now(), 0.5), after(1.0));
    }

    #[test]
    fn a_bad_token_or_missing_page_is_never_retried() {
        for status in [401, 403, 404, 500, 302] {
            assert_eq!(retry_delay(Some(status), Some("3"), 1, now(), 0.5), Retry::Stop, "{status}");
        }
    }

    #[test]
    fn only_a_limit_with_a_hint_sets_a_cooldown() {
        assert_eq!(cooldown_until(429, Some("3"), now()), Some(at("2015-10-21T07:27:53Z")));
        assert_eq!(
            cooldown_until(503, Some("Wed, 21 Oct 2015 07:28:00 GMT"), now()),
            Some(at("2015-10-21T07:28:00Z"))
        );
        assert_eq!(cooldown_until(429, None, now()), None);
        assert_eq!(cooldown_until(502, Some("3"), now()), None);
        assert_eq!(cooldown_until(429, Some("soon"), now()), None);
    }

    fn replay(name: &str) -> Http {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/replay").join(name);
        Http::with_replay(Client::new(), Some(Replay::load(&dir).unwrap()))
    }

    const BUG_1: &str = "https://bugzilla.mozilla.org/rest/bug?id=1";
    const BUG_2: &str = "https://bugzilla.mozilla.org/rest/bug?id=2";
    const JIRA: &str = "https://mozilla-hub.atlassian.net/rest/api/3/myself";

    #[test]
    fn one_limited_request_makes_every_request_to_that_service_wait_and_no_other() {
        let http = replay("cooldown");
        let start = http.clock.now();
        let (one, two, jira) = futures::executor::block_on(async {
            futures::join!(
                http.send(http.client.get(BUG_1)),
                http.send(http.client.get(BUG_2)),
                http.send(http.client.get(JIRA)),
            )
        });
        assert!(one.is_ok() && two.is_ok() && jira.is_ok(), "{one:?} {two:?} {jira:?}");
        let calls = http.take_calls();
        let limited = calls.iter().find(|c| c.status == Some(429)).expect("the recorded 429");
        let cooled = limited.at.as_millisecond() + 3000;
        let bugzilla: Vec<&Call> = calls.iter().filter(|c| c.url.contains("bugzilla")).collect();
        assert_eq!(bugzilla.len(), 3, "the 429, its retry, and the other request: {calls:#?}");
        for c in bugzilla.iter().filter(|c| c.status == Some(200)) {
            assert!(c.at.as_millisecond() >= cooled, "sent before the cooldown ended: {c:#?}");
            assert_eq!(c.wait_ms, 3000, "{c:#?}");
        }
        let jira: Vec<&Call> = calls.iter().filter(|c| c.url.contains("atlassian")).collect();
        assert_eq!(jira.len(), 1);
        assert_eq!(jira[0].wait_ms, 0, "Jira doesn't wait on Bugzilla's limit");
        assert_eq!(jira[0].at, start);
    }

    #[test]
    fn a_limit_longer_than_a_refresh_fails_the_service_at_once_with_when_to_retry() {
        let http = replay("beyond-budget");
        let until = at("2100-01-01T00:00:00Z");
        let run = |url: &str| futures::executor::block_on(http.send(http.client.get(url)));
        let first = run(BUG_1).unwrap_err();
        assert_eq!(first.retry_at(), Some(until), "{first}");
        assert!(first.to_string().starts_with("rate limited, try again after "), "{first}");
        let second = run(BUG_2).unwrap_err();
        assert_eq!(second.retry_at(), Some(until), "{second}");
        assert!(run(JIRA).is_ok());
        let calls = http.take_calls();
        let sent: Vec<&str> = calls.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(sent, [BUG_1, JIRA], "the second Bugzilla request is never sent");
    }

    #[test]
    fn reviewer_availability_requests_the_recorded_attachment_in_one_batch() {
        let http = replay("availability");
        let phab = crate::phab::Phab {
            http: &http,
            host: "https://phabricator.services.mozilla.com/api/",
            token: "fixture-token",
        };
        let found = futures::executor::block_on(phab.availability(&["PHID-USER-fixture"])).unwrap();
        assert!(found.is_empty());
        assert_eq!(http.take_calls().len(), 1);
    }

    #[test]
    fn a_request_queued_for_a_permit_rechecks_the_service_cooldown() {
        let http = replay("cooldown");
        let start = http.clock.now();
        futures::executor::block_on(async {
            let held = http.permits[Service::Bugzilla.index()].acquire_many(2).await.unwrap();
            let request = http.send(http.client.get(BUG_2));
            futures::pin_mut!(request);
            assert!(futures::poll!(&mut request).is_pending());
            http.cooldowns[Service::Bugzilla.index()].store(start.as_millisecond() + 3000, Ordering::SeqCst);
            drop(held);
            request.await.unwrap();
        });
        let calls = http.take_calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].at.as_millisecond() >= start.as_millisecond() + 3000);
    }

    #[test]
    fn a_local_wait_budget_is_not_reported_as_a_server_rate_limit() {
        let http = replay("cooldown");
        http.wait_ms.store(30_000, Ordering::SeqCst);
        let error = futures::executor::block_on(http.wait(Duration::from_secs(1), Service::Jira)).unwrap_err();
        assert!(matches!(error, Error::Api(_)), "{error}");
        assert_eq!(error.retry_at(), None);
    }

    #[test]
    fn dependency_refresh_preserves_each_services_server_retry_time() {
        let http = replay("dependency-limit");
        let mut raw: worklens_core::Raw = serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap();
        raw.bugs[0].depends_on = vec![worklens_core::BugId(1)];
        raw.revisions[0].phid = Some("PHID-DREV-fixture".into());
        let cfg = crate::fetch::Config {
            email: Some("me@example.com".into()),
            bugzilla_api_key: None,
            jira: None,
            phab: Some(crate::fetch::PhabConfig {
                host: "https://phabricator.services.mozilla.com/api/".into(),
                token: "fixture-token".into(),
            }),
            whiteboard: None,
            drift_days: 30,
            telemetry: false,
            landing_checks: false,
            repo: None,
        };
        futures::executor::block_on(crate::dependencies::refresh(&http, &cfg, &mut raw));
        for source in ["bugzilla-dependencies", "phabricator-dependencies"] {
            let error = raw.errors.iter().find(|e| e.source == source).unwrap();
            assert_eq!(error.retry_at, Some(at("2100-01-01T00:00:00Z")), "{source}");
        }
        assert_eq!(http.take_calls().len(), 2);
    }

    #[test]
    fn supplemental_refresh_preserves_rate_limits_from_each_source() {
        let http = replay("supplemental-limit");
        let mut raw: worklens_core::Raw = serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap();
        raw.telemetry = None;
        raw.landings = None;
        raw.supplemental_cache = Default::default();
        let cfg = crate::fetch::Config {
            email: Some("me@example.com".into()),
            bugzilla_api_key: None,
            jira: None,
            phab: None,
            whiteboard: None,
            drift_days: 30,
            telemetry: true,
            landing_checks: true,
            repo: None,
        };
        futures::executor::block_on(crate::supplemental::refresh(&http, &cfg, &mut raw, &[], |_| {}, now()));
        for source in ["probeinfo", "treeherder-landings"] {
            let errors: Vec<_> = raw.errors.iter().filter(|e| e.source == source).collect();
            assert!(!errors.is_empty(), "{source}");
            assert!(errors.iter().all(|e| e.retry_at == Some(at("2100-01-01T00:00:00Z"))), "{source}");
        }
    }

    #[test]
    fn supplemental_hosts_do_not_share_service_cooldowns() {
        assert_ne!(Service::of("product-details.mozilla.org"), Service::of("crash-stats.mozilla.org"));
        assert_ne!(
            Service::of("product-details.mozilla.org"),
            Service::of("probeinfo.telemetry.mozilla.org")
        );
        assert_ne!(
            Service::of("crash-stats.mozilla.org"),
            Service::of("probeinfo.telemetry.mozilla.org")
        );
    }

    #[test]
    fn repeated_server_delays_share_one_refresh_wait_budget() {
        let http = replay("wait-budget");
        let start = http.clock.now();
        let error = futures::executor::block_on(http.send(http.client.get(BUG_1))).unwrap_err();
        assert!(error.retry_at().is_some(), "{error}");
        assert!(http.clock.now().as_second() - start.as_second() <= 30);
        assert_eq!(http.take_calls().len(), 2);
    }

    #[test]
    fn a_replay_route_matches_its_path_and_the_query_it_names() {
        let route = Route {
            method: "GET".into(),
            url: reqwest::Url::parse("https://bugzilla.mozilla.org/rest/bug/*/comment?a=1").unwrap(),
            file: String::new(),
            answer: Answer::default(),
            used: AtomicU32::new(0),
        };
        let url = |u: &str| reqwest::Url::parse(u).unwrap();
        assert!(route.matches("GET", &url("https://bugzilla.mozilla.org/rest/bug/9/comment?b=2&a=1")));
        assert!(!route.matches("POST", &url("https://bugzilla.mozilla.org/rest/bug/9/comment?a=1")));
        assert!(!route.matches("GET", &url("https://bugzilla.mozilla.org/rest/bug/9/comment?a=2")));
        assert!(!route.matches("GET", &url("https://bugzilla.mozilla.org/rest/bug/9/attachment?a=1")));
        assert!(!route.matches("GET", &url("https://evil.example/rest/bug/9/comment?a=1")));
        assert!(!route.matches("GET", &url("https://bugzilla.mozilla.org/rest/bug/comment?a=1")));
        let both = Route {
            url: reqwest::Url::parse("https://bugzilla.mozilla.org/rest/bug?a=1&b=2").unwrap(),
            ..route
        };
        assert!(both.matches("GET", &url("https://bugzilla.mozilla.org/rest/bug?b=2&a=1")));
        assert!(
            !both.matches("GET", &url("https://bugzilla.mozilla.org/rest/bug?a=1")),
            "every named pair must be present"
        );
    }

    #[test]
    fn each_service_has_its_own_limit() {
        assert_eq!(Service::of("mozilla-hub.atlassian.net"), Service::of("api.atlassian.com"));
        assert_ne!(Service::of("bugzilla.mozilla.org"), Service::of("treeherder.mozilla.org"));
        assert_ne!(Service::of("phabricator.services.mozilla.com"), Service::of("bugzilla.mozilla.org"));
        assert_ne!(Service::of("product-details.mozilla.org"), Service::of("crash-stats.mozilla.org"));
    }

    proptest::proptest! {
        #![proptest_config(worklens_testkit::config(512))]

        #[test]
        fn any_retry_after_header_gives_a_bounded_answer(
            header in worklens_testkit::text(),
            status in proptest::sample::select(vec![429u16, 502, 503, 504, 500, 401]),
            attempt in 1u32..6,
            jitter in 0.0f64..=1.0,
        ) {
            match retry_delay(Some(status), Some(&header), attempt, now(), jitter) {
                Retry::After(d) => proptest::prop_assert!(d <= MAX_WAIT, "{d:?}"),
                Retry::Limited(until) => proptest::prop_assert!(until > now()),
                Retry::Stop => {}
            }
            let _ = cooldown_until(status, Some(&header), now());
        }
    }
}
