use std::fmt::{Display, Write as _};

use jiff::Timestamp;
use worklens_core::bugs::{State, days_between};
use worklens_core::drift::Level;
use worklens_core::rules::Reason;
use worklens_core::section::Section;
use worklens_core::snapshot::{ItemId, Scope, TryText, jira_read};
use worklens_core::{Entry, Snapshot, present};

pub fn no_color() -> bool {
    no_color_in(std::env::var_os("NO_COLOR").as_deref())
}

fn no_color_in(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

#[derive(Clone, Copy)]
pub struct Style {
    pub color: bool,
}

impl Style {
    pub fn detect() -> Self {
        use std::io::IsTerminal;
        Self {
            color: std::io::stdout().is_terminal() && !no_color(),
        }
    }

    fn paint(self, code: u8, text: impl Display) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn red(self, t: impl Display) -> String {
        self.paint(31, t)
    }
    pub fn green(self, t: impl Display) -> String {
        self.paint(32, t)
    }
    pub fn yellow(self, t: impl Display) -> String {
        self.paint(33, t)
    }
    pub fn cyan(self, t: impl Display) -> String {
        self.paint(36, t)
    }
    pub fn dim(self, t: impl Display) -> String {
        self.paint(90, t)
    }
    pub fn bold(self, t: impl Display) -> String {
        self.paint(1, t)
    }
}

pub fn header(st: Style, s: &Snapshot) -> String {
    let age = days_between(Some(s.raw.fetched_at), s.now) * 24.0 * 60.0;
    let mut out = st.dim(format!("{} · fetched {} min ago", s.me(), age.round()));
    for e in &s.raw.errors {
        let line = format!("{}: {}", e.source, e.message);
        let _ = write!(out, "\n{}", if e.unset { st.dim(line) } else { st.yellow(format!("! {line}")) });
    }
    out
}

pub fn sprint_line(st: Style, s: &Snapshot) -> String {
    let Some(sp) = &s.sprint else {
        return st.dim(if jira_read(&s.raw) {
            "No active sprint."
        } else {
            "No sprint: Jira wasn't read."
        });
    };
    let sprint = s.issue(sp.first).sprint.as_ref();
    let committed = format!("{}/{} pts committed", sp.committed, sp.budget);
    let mut parts = vec![
        st.bold(sprint.map_or("", |x| x.name.as_str())),
        if sp.committed > sp.budget {
            st.red(committed)
        } else {
            st.green(committed)
        },
        format!("{} done", sp.done),
    ];
    if let Some(end) = sprint.and_then(|x| x.end_date) {
        parts.push(format!("{} days left", (-days_between(Some(end), s.now)).ceil()));
    }
    if !sp.unestimated.is_empty() {
        let keys: Vec<&str> = sp.unestimated.iter().map(|i| s.issue(*i).key.0.as_str()).collect();
        parts.push(st.yellow(format!("unestimated: {}", keys.join(", "))));
    }
    parts.join(" · ")
}

pub fn train_line(st: Style, s: &Snapshot) -> Option<String> {
    let t = s.raw.trains.as_ref()?;
    let (_, days) = s.merge()?;
    let when = match days {
        ..0 => "merged".to_owned(),
        0 => "merges to Beta today".to_owned(),
        1 => format!("merges to Beta {}, tomorrow", t.next_merge.strftime("%a %b %d")),
        n => format!("merges to Beta {}, in {n} days", t.next_merge.strftime("%a %b %d")),
    };
    let when = if (0..=2).contains(&days) { st.yellow(when) } else { when };
    Some(st.dim(format!("Nightly {} · {when} · Beta {} · Release {}", t.nightly, t.beta, t.release)))
}

pub fn section_block(st: Style, s: &Snapshot, section: Section) -> String {
    let shown: Vec<_> = s.section(section, false).collect();
    let hidden = s.section(section, true).count() - shown.len();
    let mut out = format!("{} {}", st.bold(section.title()), st.dim(shown.len()));
    if hidden > 0 {
        out.push_str(&st.dim(format!(" (+{hidden} done or snoozed)")));
    }
    if shown.is_empty() {
        let _ = write!(out, "\n  {}", st.dim(s.empty_text(section)));
        return out;
    }
    for i in shown {
        let e = s.entry(i);
        let days = days_between(present::age_since(s, e), s.now).floor();
        let age = if days.is_finite() {
            format!("{:>4}", format!("{}d", days.max(0.0)))
        } else {
            "    ".to_owned()
        };
        let age = if e.placement.is_some_and(|p| p.stale) {
            st.yellow(age)
        } else {
            age
        };
        let why = match section {
            Section::Drift | Section::Team => e.drift.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; "),
            _ => {
                let mut why = e.placement.map(|_| s.reason(e).to_string()).unwrap_or_default();
                let has_try = matches!(e.placement.map(|p| p.reason), Some(Reason::Accepted(_)));
                if let Some(r) = present::try_revision(s, e).filter(|_| !has_try) {
                    if !why.is_empty() {
                        why.push_str("; ");
                    }
                    let _ = write!(why, "{}", TryText(s.try_state(r)));
                }
                why
            }
        };
        let new = if e.marks.is_new { st.cyan("•") } else { " ".to_owned() };
        let tag = format!("{:<4}", s.tag(e).to_string());
        let id = format!("{:<11}", s.id(e).to_string());
        let _ = write!(out, "\n{new}{age}  {tag} {} {}\n{:11}{}", st.bold(id), s.title(e), "", st.dim(why));
    }
    out
}

fn states(s: &Snapshot, e: &Entry) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(b) = e.bug {
        parts.push(State(s.bug(b)).to_string());
    }
    if let Some(i) = e.issue {
        parts.push(format!("{} {}", s.issue(i).key, s.issue(i).status));
    }
    parts.extend(
        e.revisions
            .iter()
            .map(|r| format!("{} {}", s.rev(*r).id, s.rev(*r).status.as_str())),
    );
    if let Some(r) = e.review {
        parts.push(format!("review {}", s.rev(r).id));
    }
    parts.join(" · ")
}

pub fn item_block(st: Style, s: &Snapshot, e: &Entry) -> String {
    let mut out = format!("{}  {}", st.bold(s.id(e)), s.title(e));
    let states = states(s, e);
    if !states.is_empty() {
        let _ = write!(out, "\n{}", st.dim(format!("    {states}")));
    }
    if let Some(p) = s.parent(e) {
        let kind = p.kind.as_deref().unwrap_or("parent").to_lowercase();
        let _ = write!(out, "\n{}", st.dim(format!("    {kind} {} {} · {}", p.key, p.summary, p.status)));
    }
    if let Some(i) = e.idea {
        let idea = s.idea(i);
        let mut parts = vec![idea.status.clone()];
        parts.extend(idea.weekly_status.clone());
        parts.extend(idea.target.clone());
        let line = if let ItemId::Idea(_) = e.id {
            let work: Vec<String> = s.work_of(i).map(|w| s.id(s.entry(w)).to_string()).collect();
            if e.scope == Scope::Team || work.is_empty() {
                format!("    {}", parts.join(" · "))
            } else {
                format!("    {} · your work: {}", parts.join(" · "), work.join(", "))
            }
        } else {
            format!("    idea {} {} · {}", idea.key, idea.summary, parts.join(" · "))
        };
        let _ = write!(out, "\n{}", st.dim(line));
    }
    if let Some(step) = s.next_step(e) {
        let _ = write!(out, "\n{}", st.cyan(format!("    → {step}")));
    }
    if let Some(p) = e.placement {
        let _ = write!(out, "\n{}", st.dim(format!("    {}: {}", p.section.title(), s.reason(e))));
    }
    let tracked = s.tracked_for(e);
    if !tracked.is_empty() {
        let _ = write!(out, "\n{}", st.yellow(format!("    tracked for {}", tracked.join(", "))));
    }
    for d in &e.drift {
        let line = if d.level == Level::Error {
            st.red(format!("    ✗ {}", d.message))
        } else {
            st.yellow(format!("    ~ {}", d.message))
        };
        let _ = write!(out, "\n{line}\n{}", st.dim(format!("      why: {}", d.cause)));
        if let Some(fix) = &d.fix {
            let _ = write!(out, "\n{}", st.dim(format!("      fix: {}", present::FixText(fix))));
        }
    }
    out
}

pub fn show(st: Style, s: &Snapshot, e: &Entry) -> String {
    let mut out = item_block(st, s, e);
    for d in worklens_core::dependencies::of(s, e) {
        let owner = d.owner.as_deref().unwrap_or("owner unknown");
        let _ = write!(out, "\n    Depends: {} · {} · {owner}\n      {}", d.id, d.status, d.url);
    }
    if !e.changes.is_empty() {
        let _ = write!(out, "\n{}", st.dim("since you last looked:"));
        for change in &e.changes {
            let _ = write!(out, "\n  {} {change}", st.cyan("•"));
        }
    }
    for line in worklens_core::evidence::lines(s, e) {
        let _ = write!(out, "\n{line}");
    }
    if let Some(c) = s.last_comment(e) {
        let excerpt: Vec<&str> = c.text.lines().take(6).collect();
        let _ = write!(
            out,
            "\n{}",
            st.dim(format!("\n    last comment, {}:\n    {}", c.author, excerpt.join("\n    ")))
        );
    }
    out
}

pub fn sprint_rows(st: Style, s: &Snapshot, entries: &[&Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        let Some(issue) = e.issue.map(|i| s.issue(i)) else { continue };
        let points = issue.points.map_or_else(|| "?".to_owned(), |p| p.to_string());
        let _ = writeln!(out, "{points:>3} pts  {:<12} {}  {}", issue.status, st.bold(s.id(e)), s.title(e));
    }
    out.truncate(out.trim_end().len());
    out
}

pub fn date(t: Timestamp) -> String {
    present::date(t, &jiff::tz::TimeZone::system())
}

#[cfg(test)]
mod no_color_tests {
    use super::no_color_in;
    use std::ffi::OsStr;

    #[test]
    fn only_a_non_empty_no_color_turns_color_off() {
        assert!(!no_color_in(None));
        assert!(!no_color_in(Some(OsStr::new(""))), "set but empty leaves color on");
        assert!(no_color_in(Some(OsStr::new("1"))));
    }
}
