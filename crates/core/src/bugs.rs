use std::fmt;

use jiff::Timestamp;

use crate::ids::{BugId, RevId};
use crate::raw::Bug;

pub const BUGZILLA: &str = "https://bugzilla.mozilla.org/show_bug.cgi?id=";
pub const JIRA_BROWSE: &str = "https://mozilla-hub.atlassian.net/browse/";
pub const PHABRICATOR: &str = "https://phabricator.services.mozilla.com/";

pub fn bug_url(id: BugId) -> String {
    format!("{BUGZILLA}{id}")
}

pub fn jira_url(key: &str) -> String {
    format!("{JIRA_BROWSE}{key}")
}

pub fn rev_url(id: RevId) -> String {
    format!("{PHABRICATOR}{id}")
}

pub fn lando_url(id: RevId) -> String {
    format!("https://lando.moz.tools/{id}/")
}

pub fn jira_key_of(bug: &Bug) -> Option<&str> {
    bug.see_also.iter().find_map(|url| jira_key_in(url))
}

fn jira_key_in(url: &str) -> Option<&str> {
    let mut segments = url.split('/');
    segments.find(|s| s.ends_with("mozilla-hub.atlassian.net"))?;
    if segments.next()? != "browse" {
        return None;
    }
    let rest = segments.next()?;
    let letters = rest.bytes().take_while(u8::is_ascii_uppercase).count();
    if letters == 0 || rest.as_bytes().get(letters) != Some(&b'-') {
        return None;
    }
    let digits = rest[letters + 1..].bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| &rest[..letters + 1 + digits])
}

pub fn crash_signatures(bug: &Bug) -> Vec<&str> {
    bug.cf_crash_signature
        .split("[@")
        .map(|s| s.trim().trim_end_matches(']').trim())
        .filter(|s| !s.is_empty())
        .collect()
}

const DEAD: [&str; 7] = ["DUPLICATE", "INVALID", "WONTFIX", "INCOMPLETE", "WORKSFORME", "MOVED", "INACTIVE"];

pub fn is_closed(bug: &Bug) -> bool {
    bug.status == "RESOLVED" || bug.status == "VERIFIED"
}

pub fn is_dead(bug: &Bug) -> bool {
    is_closed(bug) && DEAD.contains(&bug.resolution.as_str())
}

pub fn assignee(bug: &Bug) -> Option<&str> {
    (bug.assigned_to != "nobody@mozilla.org").then_some(bug.assigned_to.as_str())
}

pub struct State<'a>(pub &'a Bug);

impl fmt::Display for State<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bug = self.0;
        if bug.resolution.is_empty() {
            f.write_str(&bug.status)
        } else {
            write!(f, "{} {}", bug.status, bug.resolution)
        }
    }
}

pub fn days_between(then: Option<Timestamp>, now: Timestamp) -> f64 {
    then.map_or(f64::INFINITY, |t| (now.as_millisecond() - t.as_millisecond()) as f64 / 86_400_000.0)
}

pub fn business_days_between(then: Timestamp, now: Timestamp) -> i64 {
    use jiff::civil::Weekday;
    let utc = jiff::tz::TimeZone::UTC;
    let (mut day, end) = (then.to_zoned(utc.clone()).date(), now.to_zoned(utc).date());
    let mut n = 0;
    while day < end {
        day = day.tomorrow().unwrap_or(end);
        if !matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday) {
            n += 1;
        }
    }
    n
}

pub struct HumanDays(pub f64);

impl fmt::Display for HumanDays {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0.floor();
        if n <= 0.0 {
            f.write_str("today")
        } else if n == 1.0 {
            f.write_str("1 day")
        } else {
            write!(f, "{n} days")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jira_keys_in_urls() {
        assert_eq!(
            jira_key_in("https://mozilla-hub.atlassian.net/browse/AIPLAT-1416"),
            Some("AIPLAT-1416")
        );
        assert_eq!(jira_key_in("https://mozilla-hub.atlassian.net/browse/AI-7?focus=1"), Some("AI-7"));
        assert_eq!(jira_key_in("https://mozilla-hub.atlassian.net/browse/ai-7"), None);
        assert_eq!(jira_key_in("https://mozilla-hub.atlassian.net/browse/AI-"), None);
        assert_eq!(jira_key_in("https://github.com/x"), None);
        assert_eq!(jira_key_in("https://mozilla-hub.atlassian.net/jira/AI-7"), None);
        assert_eq!(jira_key_in("https://mozilla-hub.atlassian.net/browse/"), None);
        assert_eq!(jira_key_in("https://mozilla-hub.atlassian.net"), None);
    }

    #[test]
    fn business_days_skip_weekends() {
        let t = |s: &str| s.parse::<Timestamp>().unwrap();
        assert_eq!(business_days_between(t("2026-11-02T09:00:00Z"), t("2026-11-04T17:00:00Z")), 2);
        assert_eq!(
            business_days_between(t("2026-11-02T09:00:00Z"), t("2026-11-08T12:00:00Z")),
            4,
            "Saturday and Sunday don't count"
        );
        assert_eq!(
            business_days_between(t("2026-11-06T09:00:00Z"), t("2026-11-09T09:00:00Z")),
            1,
            "Friday to Monday"
        );
        assert_eq!(business_days_between(t("2026-11-02T09:00:00Z"), t("2026-11-02T17:00:00Z")), 0);
    }

    #[test]
    fn human_days() {
        assert_eq!(HumanDays(0.4).to_string(), "today");
        assert_eq!(HumanDays(1.9).to_string(), "1 day");
        assert_eq!(HumanDays(9.0).to_string(), "9 days");
    }
}
