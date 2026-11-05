use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::section::Section;
use crate::snapshot::Snapshot;

pub const KEEP_DAYS: usize = 60;

pub const COMPARE_DAYS: i64 = 14;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftLog {
    pub version: u32,
    pub days: Vec<DriftDay>,
}

impl Default for DriftLog {
    fn default() -> Self {
        Self {
            version: 1,
            days: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftDay {
    pub date: Date,
    pub mine: usize,
    pub team: usize,
}

pub fn counts(s: &Snapshot) -> Option<(usize, usize)> {
    if s.raw.errors.iter().any(|e| e.source == "bugzilla" || e.source == "jira") {
        return None;
    }
    Some((s.section(Section::Drift, true).count(), s.section(Section::Team, true).count()))
}

impl DriftLog {
    pub fn record(&mut self, date: Date, mine: usize, team: usize) {
        let day = DriftDay { date, mine, team };
        match self.days.binary_search_by_key(&date, |d| d.date) {
            Ok(i) => self.days[i] = day,
            Err(i) => self.days.insert(i, day),
        }
        let extra = self.days.len().saturating_sub(KEEP_DAYS);
        self.days.drain(..extra);
    }

    pub fn baseline(&self, today: Date) -> Option<DriftDay> {
        let from = today.checked_sub(jiff::Span::new().days(COMPARE_DAYS)).ok()?;
        self.days.iter().copied().find(|d| d.date >= from && d.date < today)
    }
}
