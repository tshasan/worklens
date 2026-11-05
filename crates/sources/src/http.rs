use std::time::{Duration, Instant};

use jiff::Timestamp;
use reqwest::{Client, Request, RequestBuilder};
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
}

impl Error {
    pub fn summary(&self) -> String {
        self.to_string().lines().next().unwrap_or_default().to_owned()
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

const PER_HOST: usize = 2;

use tokio::sync::Semaphore;
static BUGZILLA: Semaphore = Semaphore::const_new(PER_HOST);
static PHABRICATOR: Semaphore = Semaphore::const_new(PER_HOST);
static JIRA: Semaphore = Semaphore::const_new(PER_HOST);
static TREEHERDER: Semaphore = Semaphore::const_new(PER_HOST);
static OTHER: Semaphore = Semaphore::const_new(PER_HOST);

fn permits_for(host: &str) -> &'static Semaphore {
    match host {
        "bugzilla.mozilla.org" => &BUGZILLA,
        "phabricator.services.mozilla.com" => &PHABRICATOR,
        "api.atlassian.com" => &JIRA,
        h if h.ends_with(".atlassian.net") => &JIRA,
        "treeherder.mozilla.org" => &TREEHERDER,
        _ => &OTHER,
    }
}

pub fn parse<T: DeserializeOwned>(what: &'static str, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|source| Error::Parse { what, source })
}

pub struct Http {
    pub client: Client,
    calls: CallLog,
}

impl Http {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent("worklens (read-only)")
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            calls: CallLog::default(),
        })
    }

    pub async fn send(&self, req: RequestBuilder) -> Result<Vec<u8>> {
        let mut req = req.build()?;
        let mut attempt = 1;
        loop {
            let copy = req.try_clone();
            let (result, delay) = self.attempt(req, attempt).await;
            match (delay, copy) {
                (Some(delay), Some(copy)) => {
                    tokio::time::sleep(delay).await;
                    req = copy;
                    attempt += 1;
                }
                _ => return result,
            }
        }
    }

    async fn attempt(&self, req: Request, attempt: u32) -> (Result<Vec<u8>>, Option<Duration>) {
        let limit = permits_for(req.url().host_str().unwrap_or_default());
        let _permit = limit.acquire().await.expect("HTTP semaphores are never closed");
        let (method, url) = (req.method().to_string(), req.url().to_string());
        let (at, started) = (Timestamp::now(), Instant::now());
        let record = |status: Option<u16>, error: Option<String>| {
            let ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
            self.calls.push(Call {
                at,
                method: method.clone(),
                url: url.clone(),
                status,
                ms,
                error,
            });
        };
        let res = match self.client.execute(req).await {
            Ok(res) => res,
            Err(e) => {
                record(None, Some(e.to_string()));
                let delay = if e.is_connect() || e.is_timeout() {
                    retry_delay(None, None, attempt)
                } else {
                    None
                };
                return (Err(e.into()), delay);
            }
        };
        let status = res.status();
        record(Some(status.as_u16()), None);
        let retry_after = res
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = match res.bytes().await {
            Ok(body) => body.to_vec(),
            Err(e) => return (Err(e.into()), None),
        };
        if !status.is_success() {
            let text = String::from_utf8_lossy(&body);
            let url = url.split('?').next().unwrap_or_default().to_owned();
            let error = Error::Status {
                status: status.as_u16(),
                url,
                body: text.chars().take(300).collect(),
            };
            return (Err(error), retry_delay(Some(status.as_u16()), retry_after.as_deref(), attempt));
        }
        (Ok(body), None)
    }

    pub fn take_calls(&self) -> Vec<Call> {
        self.calls.take()
    }
}

const MAX_ATTEMPTS: u32 = 2;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

fn retry_delay(status: Option<u16>, retry_after: Option<&str>, attempt: u32) -> Option<Duration> {
    const DEFAULT: Duration = Duration::from_secs(1);
    if attempt >= MAX_ATTEMPTS {
        return None;
    }
    match status {
        None => Some(DEFAULT),
        Some(429 | 502 | 503 | 504) => {
            let asked = retry_after.and_then(|s| s.trim().parse::<u64>().ok()).map(Duration::from_secs);
            Some(asked.map_or(DEFAULT, |d| d.min(MAX_RETRY_AFTER)))
        }
        Some(_) => None,
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

    const fn secs(n: u64) -> Option<Duration> {
        Some(Duration::from_secs(n))
    }

    #[test]
    fn retries_a_busy_server_once_waiting_as_asked() {
        assert_eq!(retry_delay(Some(429), Some("3"), 1), secs(3));
        assert_eq!(retry_delay(Some(503), None, 1), secs(1));
        assert_eq!(retry_delay(Some(502), None, 1), secs(1));
        assert_eq!(retry_delay(Some(504), Some("0"), 1), secs(0));
        assert_eq!(retry_delay(Some(503), Some("Wed, 21 Oct 2015 07:28:00 GMT"), 1), secs(1));
    }

    #[test]
    fn caps_a_long_retry_after() {
        assert_eq!(retry_delay(Some(429), Some("120"), 1), secs(10));
    }

    #[test]
    fn retries_a_connect_error_or_timeout_once() {
        assert_eq!(retry_delay(None, None, 1), secs(1));
        assert_eq!(retry_delay(None, None, 2), None);
    }

    #[test]
    fn does_not_retry_other_statuses_or_a_second_failure() {
        assert_eq!(retry_delay(Some(500), None, 1), None);
        assert_eq!(retry_delay(Some(404), None, 1), None);
        assert_eq!(retry_delay(Some(302), None, 1), None);
        assert_eq!(retry_delay(Some(429), Some("3"), 2), None);
        assert_eq!(retry_delay(Some(503), None, 2), None);
    }

    #[test]
    fn each_service_has_its_own_limit() {
        let same = |a: &str, b: &str| std::ptr::eq(permits_for(a), permits_for(b));
        assert!(same("mozilla-hub.atlassian.net", "api.atlassian.com"));
        assert!(!same("bugzilla.mozilla.org", "treeherder.mozilla.org"));
        assert!(!same("phabricator.services.mozilla.com", "bugzilla.mozilla.org"));
        assert!(same("product-details.mozilla.org", "crash-stats.mozilla.org"));
    }
}
