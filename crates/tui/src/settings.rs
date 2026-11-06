use worklens_core::section::Section;

use crate::app::Pane;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Show {
    #[default]
    Auto,
    Always,
    Hidden,
}

impl Show {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Always, Self::Hidden];

    pub const fn word(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Hidden => "hidden",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.word() == text.trim())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeName {
    #[default]
    Auto,
    Dark,
    Light,
    Plain,
    NoColor,
}

impl ThemeName {
    pub const ALL: [Self; 5] = [Self::Auto, Self::Dark, Self::Light, Self::Plain, Self::NoColor];

    pub const fn word(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Plain => "plain",
            Self::NoColor => "nocolor",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.word() == text.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Whiteboard,
    SprintBudget,
    Telemetry,
    LandingChecks,
    DriftDays,
    CacheMinutes,
    Theme,
    ShowHidden,
    Agent,
    AgentDir,
    Section(usize),
    Pane(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Toggle,
    Text,
    Fixed,
}

pub fn groups() -> Vec<(&'static str, Vec<Setting>)> {
    use Setting as S;
    vec![
        ("Sprint", vec![S::Whiteboard, S::SprintBudget]),
        ("Fetching", vec![S::Telemetry, S::LandingChecks, S::DriftDays, S::CacheMinutes]),
        ("Look", vec![S::Theme, S::ShowHidden]),
        ("Agent", vec![S::Agent, S::AgentDir]),
        (
            "Section tabs",
            (0..Section::COUNT)
                .filter(|i| *i != Section::Backlog.index())
                .map(S::Section)
                .collect(),
        ),
        ("Detail tabs", (0..Pane::COUNT).map(S::Pane).collect()),
    ]
}

pub fn line_of(index: usize) -> usize {
    let mut line = 0;
    let mut seen = 0;
    for (g, (_, group)) in groups().into_iter().enumerate() {
        line += usize::from(g > 0) + 1;
        if index < seen + group.len() {
            return line + index - seen;
        }
        seen += group.len();
        line += group.len();
    }
    line
}

pub fn all() -> Vec<Setting> {
    groups().into_iter().flat_map(|(_, s)| s).collect()
}

impl Setting {
    pub fn label(self) -> &'static str {
        match self {
            Self::Whiteboard => "Whiteboard",
            Self::SprintBudget => "Sprint budget",
            Self::Telemetry => "Telemetry",
            Self::LandingChecks => "Landing checks",
            Self::DriftDays => "Drift window",
            Self::CacheMinutes => "Refresh after",
            Self::Theme => "Theme",
            Self::ShowHidden => "Done & snoozed",
            Self::Agent => "Command",
            Self::AgentDir => "Directory",
            Self::Section(i) => Section::ALL[i].title(),
            Self::Pane(i) => Pane::ALL[i].title(),
        }
    }

    pub fn note(self, prefs: &Prefs) -> &'static str {
        let show = match self {
            Self::Section(i) => prefs.sections[i],
            Self::Pane(i) => prefs.panes[i],
            _ => Show::Auto,
        };
        match self {
            Self::Whiteboard => "your team's tag; Team drift is off without one",
            Self::SprintBudget => "points you plan to take per sprint",
            Self::Telemetry => "metric definitions that name your bugs",
            Self::LandingChecks => "Treeherder results for your try pushes",
            Self::DriftDays => "how far back Team drift looks",
            Self::CacheMinutes => "opening worklens refetches older data",
            Self::Theme => "auto asks the terminal for its background",
            Self::ShowHidden => "when worklens opens; Z toggles it",
            Self::Agent => "auto looks for claude, then codex",
            Self::AgentDir => "where agents and checkouts run",
            Self::Section(0) | Self::Pane(0) => "everything else falls back here",
            Self::Section(_) if show == Show::Always => "shown even when empty",
            Self::Section(_) if show == Show::Hidden => "never shown; its count shows top right",
            Self::Section(_) => "shown when it has work",
            Self::Pane(_) if show == Show::Always => "shown on every item",
            Self::Pane(_) if show == Show::Hidden => "never shown",
            Self::Pane(i) => match Pane::ALL[i] {
                Pane::Timeline => "shown when the item has dated history",
                Pane::Comments => "shown when the item has a bug",
                Pane::Links => "shown when the item has links",
                Pane::Drift => "shown when Bugzilla and Jira disagree",
                _ => "shown when a metric names the bug",
            },
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Self::Section(0) | Self::Pane(0) => Kind::Fixed,
            Self::Telemetry | Self::LandingChecks | Self::Theme | Self::ShowHidden | Self::Section(_) | Self::Pane(_) => Kind::Toggle,
            Self::Whiteboard | Self::SprintBudget | Self::DriftDays | Self::CacheMinutes | Self::Agent | Self::AgentDir => Kind::Text,
        }
    }

    pub fn refetches(self) -> bool {
        matches!(self, Self::Whiteboard | Self::Telemetry | Self::LandingChecks | Self::DriftDays)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Whiteboard => "whiteboard",
            Self::SprintBudget => "sprintBudget",
            Self::Telemetry => "telemetry",
            Self::LandingChecks => "landingChecks",
            Self::DriftDays => "driftDays",
            Self::CacheMinutes => "cacheMinutes",
            Self::Theme => "theme",
            Self::ShowHidden => "showHidden",
            Self::Agent => "agent",
            Self::AgentDir => "agentDir",
            Self::Section(_) => "sectionTabs",
            Self::Pane(_) => "detailTabs",
        }
    }

    pub fn tab_key(self) -> Option<&'static str> {
        match self {
            Self::Section(i) => Some(Section::ALL[i].id()),
            Self::Pane(i) => Some(Pane::ALL[i].id()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub whiteboard: String,
    pub sprint_budget: f32,
    pub telemetry: bool,
    pub landing_checks: bool,
    pub drift_days: i64,
    pub cache_minutes: f64,
    pub theme: ThemeName,
    pub show_hidden: bool,
    pub agent: String,
    pub agent_dir: String,
    pub sections: [Show; Section::COUNT],
    pub panes: [Show; Pane::COUNT],
}

impl Default for Prefs {
    fn default() -> Self {
        let mut sections = [Show::Auto; Section::COUNT];
        sections[0] = Show::Always;
        let mut panes = [Show::Auto; Pane::COUNT];
        panes[0] = Show::Always;
        Self {
            whiteboard: String::new(),
            sprint_budget: worklens_core::CoreConfig::default().sprint_budget,
            telemetry: true,
            landing_checks: true,
            drift_days: 30,
            cache_minutes: 10.0,
            theme: ThemeName::Auto,
            show_hidden: false,
            agent: String::new(),
            agent_dir: "~/firefox".to_owned(),
            sections,
            panes,
        }
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn plural(n: impl std::fmt::Display, unit: &str) -> String {
    let n = n.to_string();
    if n == "1" { format!("1 {unit}") } else { format!("{n} {unit}s") }
}

fn next<T: Copy + PartialEq>(all: &[T], now: T, by: isize) -> T {
    let i = all.iter().position(|x| *x == now).unwrap_or(0) as isize;
    all[(i + by).rem_euclid(all.len() as isize) as usize]
}

impl Prefs {
    pub fn value(&self, s: Setting) -> String {
        match s {
            Setting::Whiteboard if self.whiteboard.is_empty() => "none".to_owned(),
            Setting::Whiteboard => self.whiteboard.clone(),
            Setting::SprintBudget => format!("{} pts", self.sprint_budget),
            Setting::Telemetry => on_off(self.telemetry).to_owned(),
            Setting::LandingChecks => on_off(self.landing_checks).to_owned(),
            Setting::DriftDays => plural(self.drift_days, "day"),
            Setting::CacheMinutes => plural(self.cache_minutes, "minute"),
            Setting::Theme => self.theme.word().to_owned(),
            Setting::ShowHidden => if self.show_hidden { "shown" } else { "hidden" }.to_owned(),
            Setting::Agent if self.agent.is_empty() => "auto".to_owned(),
            Setting::Agent => self.agent.clone(),
            Setting::AgentDir => self.agent_dir.clone(),
            Setting::Section(i) => self.sections[i].word().to_owned(),
            Setting::Pane(i) => self.panes[i].word().to_owned(),
        }
    }

    pub fn enter_hint(&self, s: Setting) -> String {
        match s.kind() {
            Kind::Fixed => "can't be changed".to_owned(),
            Kind::Text => "edit".to_owned(),
            Kind::Toggle => {
                let mut after = self.clone();
                after.step(s, 1);
                format!("→ {}", after.value(s))
            }
        }
    }

    pub fn changed(&self, s: Setting) -> bool {
        self.value(s) != Self::default().value(s)
    }

    pub fn text(&self, s: Setting) -> String {
        match s {
            Setting::Whiteboard => self.whiteboard.clone(),
            Setting::SprintBudget => self.sprint_budget.to_string(),
            Setting::DriftDays => self.drift_days.to_string(),
            Setting::CacheMinutes => self.cache_minutes.to_string(),
            Setting::Agent => self.agent.clone(),
            Setting::AgentDir => self.agent_dir.clone(),
            _ => self.value(s),
        }
    }

    pub fn step(&mut self, s: Setting, by: isize) -> bool {
        match s {
            Setting::Telemetry => self.telemetry = !self.telemetry,
            Setting::LandingChecks => self.landing_checks = !self.landing_checks,
            Setting::ShowHidden => self.show_hidden = !self.show_hidden,
            Setting::Theme => self.theme = next(&ThemeName::ALL, self.theme, by),
            Setting::Section(i) if i > 0 => self.sections[i] = next(&Show::ALL, self.sections[i], by),
            Setting::Pane(i) if i > 0 => self.panes[i] = next(&Show::ALL, self.panes[i], by),
            _ => return false,
        }
        true
    }

    pub fn set_text(&mut self, s: Setting, text: &str) -> Result<(), String> {
        let text = text.trim();
        match s {
            Setting::Whiteboard => self.whiteboard = text.to_owned(),
            Setting::SprintBudget => {
                self.sprint_budget = text
                    .parse::<f32>()
                    .ok()
                    .filter(|n| n.is_finite() && *n > 0.0)
                    .ok_or("Sprint budget takes a number of points above 0, like 10")?;
            }
            Setting::DriftDays => {
                self.drift_days = text
                    .parse::<i64>()
                    .ok()
                    .filter(|n| (1..=365).contains(n))
                    .ok_or("Drift window takes whole days from 1 to 365, like 30")?;
            }
            Setting::CacheMinutes => {
                self.cache_minutes = text
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .ok_or("Refresh after takes minutes, 0 or more, like 10")?;
            }
            Setting::Agent => self.agent = text.to_owned(),
            Setting::AgentDir if text.is_empty() => return Err("Directory needs a path, like ~/firefox".to_owned()),
            Setting::AgentDir => self.agent_dir = text.to_owned(),
            _ => return Err(format!("{} isn't typed in; Enter changes it", s.label())),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_unchanged_and_a_step_marks_a_change() {
        let mut p = Prefs::default();
        assert!(all().into_iter().all(|s| !p.changed(s)));
        assert!(p.step(Setting::LandingChecks, 1));
        assert!(p.changed(Setting::LandingChecks));
        assert_eq!(p.value(Setting::LandingChecks), "off");
    }

    #[test]
    fn tabs_cycle_auto_always_hidden_except_the_first() {
        let mut p = Prefs::default();
        let team = Setting::Section(Section::Team.index());
        let words: Vec<String> = (0..3)
            .map(|_| {
                p.step(team, 1);
                p.value(team)
            })
            .collect();
        assert_eq!(words, ["always", "hidden", "auto"]);
        p.step(team, -1);
        assert_eq!(p.value(team), "hidden", "left goes back");
        assert!(!p.step(Setting::Section(0), 1));
        assert!(!p.step(Setting::Pane(0), 1));
        assert_eq!(Setting::Section(0).kind(), Kind::Fixed);
    }

    #[test]
    fn typed_values_are_checked_and_keep_their_unit() {
        let mut p = Prefs::default();
        assert!(p.set_text(Setting::SprintBudget, "abc").is_err());
        assert!(p.set_text(Setting::DriftDays, "0").is_err());
        assert_eq!(p.value(Setting::SprintBudget), "10 pts");
        p.set_text(Setting::SprintBudget, " 12.5 ").unwrap();
        p.set_text(Setting::CacheMinutes, "1").unwrap();
        assert_eq!(p.value(Setting::SprintBudget), "12.5 pts");
        assert_eq!(p.value(Setting::CacheMinutes), "1 minute");
        p.set_text(Setting::Whiteboard, "").unwrap();
        assert_eq!(p.value(Setting::Whiteboard), "none");
    }
}
