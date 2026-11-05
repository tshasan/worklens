use jiff::civil::Date;
use serde::Deserialize;
use worklens_core::raw::Trains;

use crate::http::{Error, Http, Result, parse};

pub const PRODUCT_DETAILS: &str = "https://product-details.mozilla.org/1.0/firefox_versions.json";

pub async fn trains(http: &Http) -> Result<Trains> {
    parse_trains(&http.send(http.client.get(PRODUCT_DETAILS)).await?)
}

fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

pub fn parse_trains(bytes: &[u8]) -> Result<Trains> {
    #[derive(Deserialize)]
    #[expect(non_snake_case, reason = "product-details' own field names")]
    struct Versions {
        FIREFOX_NIGHTLY: String,
        LATEST_FIREFOX_DEVEL_VERSION: String,
        LATEST_FIREFOX_VERSION: String,
        #[serde(default)]
        FIREFOX_ESR: String,
        #[serde(default)]
        FIREFOX_ESR_NEXT: String,
        #[serde(default)]
        FIREFOX_ESR115: String,
        NEXT_MERGE_DATE: Date,
        NEXT_RELEASE_DATE: Date,
    }
    let v: Versions = parse("product-details", bytes)?;
    let missing = |what: &str| Error::Api(format!("product-details: no {what} version"));
    let mut esr: Vec<u32> = [&v.FIREFOX_ESR_NEXT, &v.FIREFOX_ESR, &v.FIREFOX_ESR115]
        .iter()
        .filter_map(|x| major(x))
        .collect();
    esr.sort_unstable_by(|a, b| b.cmp(a));
    esr.dedup();
    Ok(Trains {
        nightly: major(&v.FIREFOX_NIGHTLY).ok_or_else(|| missing("nightly"))?,
        beta: major(&v.LATEST_FIREFOX_DEVEL_VERSION).ok_or_else(|| missing("beta"))?,
        release: major(&v.LATEST_FIREFOX_VERSION).ok_or_else(|| missing("release"))?,
        esr,
        next_merge: v.NEXT_MERGE_DATE,
        next_release: v.NEXT_RELEASE_DATE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_dates() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/product-details.json")).unwrap();
        let t = parse_trains(&bytes).unwrap();
        assert_eq!((t.nightly, t.beta, t.release), (161, 160, 159));
        assert_eq!(t.esr, [153, 115]);
        assert_eq!(t.next_merge.to_string(), "2026-11-05");
    }
}
