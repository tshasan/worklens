use jiff::civil::Date;

use crate::bugs::HumanDays;
use crate::drift::{Drift, Level};
use crate::ids::{IdeaIdx, JiraKey};
use crate::raw::{Idea, Issue, Raw, StatusCategory};
use crate::snapshot::Scope;

pub const MAX_DEPTH: usize = 4;

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn lookup<'a>(raw: &'a Raw, key: &JiraKey) -> Option<&'a Issue> {
    raw.roadmap.parents.get(key).or_else(|| raw.issues.get(key))
}

pub fn parent_of<'a>(raw: &'a Raw, issue: &Issue) -> Option<&'a Issue> {
    lookup(raw, issue.parent.as_ref()?)
}

pub fn idea_of(raw: &Raw, issue: &Issue) -> Option<IdeaIdx> {
    let mut at = Some(issue);
    for _ in 0..MAX_DEPTH {
        let i = at?;
        if let Some(idx) = i.ideas.iter().find_map(|k| raw.roadmap.ideas.get_index_of(k)) {
            return Some(IdeaIdx::new(idx));
        }
        at = parent_of(raw, i);
    }
    None
}

pub fn target_date(target: &str, today: Date) -> Option<Date> {
    let inside = target.split_once('(')?.1.split_once(')')?.0;
    let mut words = inside.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty());
    let month = words.find_map(|w| {
        let w = w.to_ascii_lowercase();
        MONTHS.iter().position(|m| w.starts_with(m))
    })?;
    let month = i8::try_from(month + 1).ok()?;
    let day: i8 = words.next()?.parse().ok()?;
    if let Some(year) = words.next().and_then(|w| w.parse::<i16>().ok()) {
        return Date::new(year, month, day).ok();
    }
    let distance = |d: &Date| today.until(*d).map_or(i32::MAX, |span| span.get_days().abs());
    [today.year() - 1, today.year(), today.year() + 1]
        .into_iter()
        .filter_map(|y| Date::new(y, month, day).ok())
        .min_by_key(distance)
}

#[derive(Clone)]
pub struct Work {
    pub label: String,
    pub status: String,
    pub open: bool,
}

pub fn open_work(work: &[Work], whose: Scope) -> Option<String> {
    let mut open = work.iter().filter(|w| w.open);
    let first = open.next()?;
    let whose = match whose {
        Scope::Mine => "yours",
        Scope::Team => "the team's",
    };
    Some(match open.count() {
        0 => format!("{} is still {}", first.label, first.status),
        1 => format!("{} is still {}, and 1 more of {whose} is open", first.label, first.status),
        n => format!("{} is still {}, and {n} more of {whose} are open", first.label, first.status),
    })
}

pub fn drift_of(idea: &Idea, work: &[Work], whose: Scope, today: Date) -> Vec<Drift> {
    let mut out = Vec::new();
    let Some(cause) = open_work(work, whose) else { return out };
    if idea.status_category == StatusCategory::Done {
        out.push(Drift {
            level: Level::Warn,
            message: format!("{} is {}", idea.key, idea.status),
            cause,
            fix: None,
        });
        return out;
    }
    let passed = idea
        .target
        .as_deref()
        .and_then(|t| Some((t, target_date(t, today)?)))
        .filter(|(_, date)| *date < today);
    if let Some((target, date)) = passed {
        let days = date.until(today).map_or(0, |span| span.get_days());
        out.push(Drift {
            level: Level::Warn,
            message: format!("{} targets {target}, {} ago", idea.key, HumanDays(f64::from(days))),
            cause,
            fix: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn idea(status: &str, category: StatusCategory, target: Option<&str>) -> Idea {
        Idea {
            key: JiraKey("AIDI-79".into()),
            summary: "Example idea: page extraction".into(),
            status: status.into(),
            status_category: category,
            assignee: None,
            updated: "2026-11-04T00:00:00Z".parse().unwrap(),
            weekly_status: Some("Dev on track".into()),
            target: target.map(str::to_owned),
            quarter: None,
            theme: None,
            priority: None,
            team: None,
        }
    }

    fn work(label: &str, status: &str, open: bool) -> Work {
        Work {
            label: label.into(),
            status: status.into(),
            open,
        }
    }

    #[test]
    fn target_dates_take_the_nearest_year() {
        let today = date(2026, 11, 4);
        assert_eq!(target_date("Fx158.0.1 (Oct 20)", today), Some(date(2026, 10, 20)));
        assert_eq!(target_date("Fx160 (Jan 12)", today), Some(date(2027, 1, 12)));
        assert_eq!(target_date("Fx150 (Sept 3, 2025)", today), Some(date(2025, 9, 3)));
        assert_eq!(target_date("Fx158", today), None);
        assert_eq!(target_date("Fx157 Off-train (by Oct 12)", today), Some(date(2026, 10, 12)));
        assert_eq!(target_date("TBD (soon)", today), None);
    }

    #[test]
    fn a_passed_target_with_open_work_is_drift() {
        let today = date(2026, 11, 4);
        let i = idea("Developing", StatusCategory::Indeterminate, Some("Fx158.0.1 (Oct 20)"));
        let d = drift_of(
            &i,
            &[work("AIPLAT-1416", "ENG REVIEW", true), work("bug 1", "FIXED", false)],
            Scope::Mine,
            today,
        );
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].message, "AIDI-79 targets Fx158.0.1 (Oct 20), 15 days ago");
        assert_eq!(d[0].cause, "AIPLAT-1416 is still ENG REVIEW");
        assert!(
            drift_of(&i, &[work("bug 1", "FIXED", false)], Scope::Mine, today).is_empty(),
            "only open work counts"
        );
        assert!(drift_of(&i, &[work("AIPLAT-1416", "ENG REVIEW", true)], Scope::Mine, date(2026, 10, 19)).is_empty());
    }

    #[test]
    fn a_released_idea_with_open_work_is_drift() {
        let i = idea("Released", StatusCategory::Done, Some("Fx158.0.1 (Oct 20)"));
        let open = [
            work("bug 1", "NEW", true),
            work("AIPLAT-2", "In Progress", true),
            work("AIPLAT-3", "To Do", true),
        ];
        let d = drift_of(&i, &open, Scope::Team, date(2026, 11, 4));
        assert_eq!(d.len(), 1, "released says it all; the passed target would repeat it");
        assert_eq!(d[0].message, "AIDI-79 is Released");
        assert_eq!(d[0].cause, "bug 1 is still NEW, and 2 more of the team's are open");
    }
}
