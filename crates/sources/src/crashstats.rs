use jiff::civil::Date;
use serde::Deserialize;

use crate::http::{Http, Result, parse};

pub const SUPERSEARCH: &str = "https://crash-stats.mozilla.org/api/SuperSearch/";

pub async fn nightly_crashes(http: &Http, signature: &str, since: Date) -> Result<u32> {
    let build = format!(">={}000000", since.strftime("%Y%m%d"));
    let exact = format!("={signature}");
    let req = http.client.get(SUPERSEARCH).query(&[
        ("product", "Firefox"),
        ("release_channel", "nightly"),
        ("build_id", build.as_str()),
        ("signature", exact.as_str()),
        ("_results_number", "0"),
    ]);
    parse_total(&http.send(req).await?)
}

pub fn parse_total(bytes: &[u8]) -> Result<u32> {
    #[derive(Deserialize)]
    struct Page {
        total: u32,
    }
    Ok(parse::<Page>("crash-stats", bytes)?.total)
}

#[cfg(test)]
mod tests {
    #[test]
    fn total() {
        assert_eq!(
            super::parse_total(br#"{"hits":[],"total":12,"facets":{},"errors":[]}"#).unwrap(),
            12
        );
    }
}
