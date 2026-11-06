use std::fmt::Write as _;
use std::path::PathBuf;

use jiff::{Timestamp, ToSpan};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use worklens_core::bugs::{bug_url, jira_url, rev_url};
use worklens_core::drift::Side;
use worklens_core::key::{EntryKey, Whereabouts};
use worklens_core::marks::{self, Hidden, Marks};
use worklens_core::present::{self, LinkKind};
use worklens_core::raw::Raw;
use worklens_core::section::Section;
use worklens_core::snapshot::ItemId;
use worklens_core::timeline;
use worklens_core::{BugId, CoreConfig, Entry, EntryIdx, RevId, Snapshot, brief, build_snapshot};

use crate::keys::{self, Action};
use crate::settings::{self, Kind, Prefs, Setting, Show};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub title: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Agent {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CustomKey {
    pub key: char,
    pub label: String,
    pub run: String,
    pub terminal: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Touched {
    pub id: String,
    pub bug: Option<BugId>,
    pub revisions: Vec<RevId>,
    pub jira: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub required: bool,
    pub detail: String,
    pub fix: Option<String>,
    pub pending: bool,
}

#[derive(Debug)]
pub enum Effect {
    Refresh,
    Refetch(Vec<Touched>),
    Open(String),
    Copy { text: String, done: String },
    SaveMarks,
    RecordDrift(worklens_core::drift_log::DriftDay),
    RunChecks,
    RunInTerminal { command: Command, pause: bool },
    RunQuietly { command: Command, label: String },
    SaveSetting { setting: Setting, done: String },
    Quit,
}

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    Focus(bool),
    Tick,
    Progress(String),
    Fetched(Result<Box<Raw>, String>),
    Done(Result<String, String>),
    Checks(Vec<Check>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    List,
    Menu,
    Filter,
    Help,
    Setup,
    Log,
    Snooze,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Sources,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Overview,
    Timeline,
    Comments,
    Links,
    Drift,
    Telemetry,
}

impl Pane {
    pub const ALL: [Self; 6] = [
        Self::Overview,
        Self::Timeline,
        Self::Comments,
        Self::Links,
        Self::Drift,
        Self::Telemetry,
    ];
    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Timeline => "timeline",
            Self::Comments => "comments",
            Self::Links => "links",
            Self::Drift => "drift",
            Self::Telemetry => "telemetry",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Timeline => "Timeline",
            Self::Comments => "Comments",
            Self::Links => "Links",
            Self::Drift => "Drift",
            Self::Telemetry => "Telemetry",
        }
    }

    fn step(self, by: isize) -> Self {
        let i = Self::ALL.iter().position(|p| *p == self).unwrap_or(0) as isize;
        Self::ALL[(i + by).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    until: Timestamp,
}

pub struct ItemAction {
    pub key: Option<char>,
    pub label: String,
    kind: ActionKind,
}

enum ActionKind {
    Open(String),
    CopyBrief,
    CopyId,
    Checkout(RevId),
    Review,
    CopyReviewPrompt,
    FixDrift {
        value: String,
        url: Option<String>,
        field: &'static str,
    },
    StartAgent,
    Done,
    Snooze,
    Custom(usize),
}

pub struct Options {
    pub cache_minutes: Option<f64>,
    pub agent: Option<Agent>,
    pub dir: PathBuf,
    pub keys: Vec<CustomKey>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Unknown,
    Dark,
    Light,
    NoColor,
}

pub const WIDE: u16 = 110;
const SEEN_AFTER_MS: i64 = 800;
const TOAST_SECONDS: i64 = 3;

pub struct App {
    pub cfg: CoreConfig,
    pub opts: Options,
    pub snapshot: Option<Snapshot>,
    pub marks: Marks,
    pub mode: Mode,
    pub tab: usize,
    cursors: [usize; Section::COUNT],
    selected_ids: [Option<EntryKey>; Section::COUNT],
    seen: [Vec<EntryKey>; Section::COUNT],
    pub filter: String,
    pub show_hidden: bool,
    pub prefs: Prefs,
    pub page: Page,
    pub setting: usize,
    pub editing: Option<String>,
    pub pane: Pane,
    pub pane_scroll: u16,
    pub menu_cursor: usize,
    pub menu_scroll: u16,
    pub log_offset: usize,
    pub help_scroll: u16,
    pub snooze_text: String,
    snooze_for: Option<EntryKey>,
    pub busy: Option<String>,
    pub error: Option<String>,
    pub action_error: Option<String>,
    pub checks: Option<Vec<Check>>,
    pub toast: Option<Toast>,
    pub since: Option<(Timestamp, timeline::Since)>,
    pressed: bool,
    pub frame: usize,
    pub size: (u16, u16),
    pub tone: Tone,
    pub now: Timestamp,
    pub rows: Vec<EntryIdx>,
    pub counts: [usize; Section::COUNT],
    touched: Vec<Touched>,
    resting: Option<(EntryKey, Timestamp)>,
    last_fetch_started: Option<Timestamp>,
    pub drift_log: worklens_core::drift_log::DriftLog,
    pub dirty: bool,
}

impl App {
    pub fn new(cfg: CoreConfig, opts: Options, raw: Option<Raw>, marks: Marks, now: Timestamp) -> Self {
        let mut app = Self {
            cfg,
            opts,
            snapshot: None,
            marks,
            mode: Mode::List,
            tab: 0,
            cursors: [0; Section::COUNT],
            selected_ids: std::array::from_fn(|_| None),
            seen: std::array::from_fn(|_| Vec::new()),
            filter: String::new(),
            show_hidden: false,
            prefs: Prefs::default(),
            page: Page::Sources,
            setting: 0,
            editing: None,
            pane: Pane::Overview,
            pane_scroll: 0,
            menu_cursor: 0,
            menu_scroll: 0,
            log_offset: 0,
            help_scroll: 0,
            snooze_text: String::new(),
            snooze_for: None,
            busy: None,
            error: None,
            action_error: None,
            checks: None,
            toast: None,
            since: None,
            pressed: false,
            frame: 0,
            size: (80, 24),
            tone: Tone::Unknown,
            now,
            rows: Vec::new(),
            counts: [0; Section::COUNT],
            touched: Vec::new(),
            resting: None,
            last_fetch_started: None,
            drift_log: worklens_core::drift_log::DriftLog::default(),
            dirty: true,
        };
        if let Some(raw) = raw {
            app.set_raw(raw, now);
        }
        app
    }

    pub fn start(&mut self, now: Timestamp) -> Vec<Effect> {
        let Some(max) = self.opts.cache_minutes else { return Vec::new() };
        let stale = self.snapshot.as_ref().is_none_or(|s| minutes_between(s.raw.fetched_at, now) >= max);
        if stale { self.begin_refresh(now) } else { Vec::new() }
    }

    pub fn section(&self) -> Section {
        Section::ALL[self.tab]
    }

    pub fn cursor(&self) -> usize {
        self.cursors[self.tab].min(self.rows.len().saturating_sub(1))
    }

    pub fn selected(&self) -> Option<&Entry> {
        selected_in(self.snapshot.as_ref(), &self.rows, self.cursor())
    }

    pub fn checks_pending(&self) -> bool {
        self.mode == Mode::Setup && self.checks.as_ref().is_some_and(|c| c.iter().any(|c| c.pending))
    }

    pub fn wide(&self) -> bool {
        self.size.0 >= WIDE
    }

    pub fn list_rows(&self) -> usize {
        let body = self.size.1.saturating_sub(4).max(6) as usize;
        if self.wide() {
            body - 2
        } else {
            (body * 35 / 100).saturating_sub(2).max(3)
        }
    }

    fn set_raw(&mut self, raw: Raw, now: Timestamp) {
        let mut s = build_snapshot(raw, &self.cfg, now);
        marks::upgrade(&mut self.marks, &s);
        if self.marks.last_look.is_none() && self.marks.items.is_empty() {
            marks::mark_all_seen(&mut self.marks, &s, now);
        }
        marks::apply(&mut s, &self.marks, now);
        if !self.pressed {
            self.since = self.marks.last_look.map(|at| (at, timeline::since(&s)));
        }
        let before = self.seen_now();
        self.snapshot = Some(s);
        self.error = None;
        let gone = self.rebuild_rows(&before);
        let menu = self.mode == Mode::Menu;
        if menu {
            self.mode = Mode::List;
        }
        let snooze = self.mode == Mode::Snooze && gone.as_ref().is_some_and(|g| self.snooze_for.as_ref() == Some(&g.key));
        if snooze {
            self.mode = Mode::List;
            self.snooze_for = None;
        }
        if let Some(gone) = gone {
            let closed = if menu {
                "; actions closed"
            } else if snooze {
                "; snooze canceled"
            } else {
                ""
            };
            let text = self.left_text(&gone, closed);
            self.flash(text, snooze, now);
        } else if menu {
            self.flash("Work refreshed; Enter reopens actions for the selected item", false, now);
        }
    }

    fn marks_changed(&mut self, now: Timestamp) -> Vec<Effect> {
        if let Some(s) = self.snapshot.as_mut() {
            marks::apply(s, &self.marks, now);
        }
        self.refresh_rows();
        vec![Effect::SaveMarks]
    }

    fn seen_now(&self) -> Vec<EntryKey> {
        let Some(s) = &self.snapshot else { return Vec::new() };
        self.rows.iter().map(|i| EntryKey::of(s, s.entry(*i))).collect()
    }

    fn refresh_rows(&mut self) {
        let before = self.seen_now();
        self.rebuild_rows(&before);
    }

    fn left_text(&self, gone: &Gone, closed: &str) -> String {
        let here = self.section();
        let id = &gone.key;
        let Some(s) = self.snapshot.as_ref() else { return String::new() };
        let filtered = s.section(here, self.show_hidden).any(|i| id.is(s, s.entry(i)));
        let mut text = match id.find(s).map(|i| s.entry(i).whereabouts()) {
            None => format!("{id} is no longer in your work"),
            Some(_) if filtered => format!("{id} no longer matches the filter"),
            Some(Whereabouts::Hidden(Hidden::Done)) => format!("{id} is done locally; Z shows it"),
            Some(Whereabouts::Hidden(Hidden::Snoozed)) => format!("{id} is snoozed locally; Z shows it"),
            Some(Whereabouts::In(other)) if other != here => format!("{id} is in {} now", other.title()),
            Some(Whereabouts::In(_) | Whereabouts::Unlisted) => format!("{id} left {}", here.title()),
        };
        text.push_str(closed);
        match self.selected() {
            Some(e) => {
                let _ = write!(text, "; selected {}", EntryKey::of(s, e));
                if !gone.neighbor {
                    text.push_str(", new here");
                }
            }
            None => text.push_str("; nothing else here"),
        }
        text
    }

    fn rebuild_rows(&mut self, before: &[EntryKey]) -> Option<Gone> {
        self.rows.clear();
        let Some(s) = &self.snapshot else {
            self.counts = [0; Section::COUNT];
            return None;
        };
        let words: Vec<String> = self.filter.to_lowercase().split_whitespace().map(str::to_owned).collect();
        let mut hay = String::new();
        let mut matches = |e: &Entry| {
            if words.is_empty() {
                return true;
            }
            hay.clear();
            let _ = write!(hay, "{} {}", s.id(e), s.title(e));
            if let Some(i) = e.issue {
                let _ = write!(hay, " {}", s.issue(i).key);
            }
            if let Some(b) = e.bug {
                let bug = s.bug(b);
                let _ = write!(hay, " {} {}", bug.component, bug.assigned_to);
            }
            for r in &e.revisions {
                let _ = write!(hay, " {}", s.rev(*r).id);
            }
            hay = hay.to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        };
        for (n, section) in Section::ALL.iter().enumerate() {
            let mut count = 0;
            for i in s
                .section(*section, self.show_hidden)
                .chain(s.section(Section::Backlog, self.show_hidden).filter(|_| *section == Section::Other))
            {
                if matches(s.entry(i)) {
                    count += 1;
                    if n == self.tab {
                        self.rows.push(i);
                    }
                }
            }
            self.counts[n] = count;
        }
        let rows = &self.rows;
        let find = |key: &EntryKey| rows.iter().position(|i| key.is(s, s.entry(*i)));
        let mut left = None;
        if let Some(wanted) = &self.selected_ids[self.tab] {
            if let Some(at) = find(wanted) {
                self.cursors[self.tab] = at;
            } else if let Some(was) = before.iter().position(|x| x == wanted) {
                let after = before[was + 1..].iter();
                let prior = before[..was].iter().rev();
                let neighbor = after.chain(prior).find_map(find);
                self.cursors[self.tab] = neighbor.unwrap_or(0);
                left = Some(Gone {
                    key: wanted.clone(),
                    neighbor: neighbor.is_some(),
                });
            }
        }
        if let Some(e) = self.selected() {
            self.selected_ids[self.tab] = Some(EntryKey::of(s, e));
        }
        left
    }

    fn flash(&mut self, text: impl Into<String>, error: bool, now: Timestamp) {
        self.toast = Some(Toast {
            text: text.into(),
            error,
            until: now + TOAST_SECONDS.seconds(),
        });
    }

    fn begin_refresh(&mut self, now: Timestamp) -> Vec<Effect> {
        if self.busy.is_some() {
            return Vec::new();
        }
        self.busy = Some("Refreshing".into());
        self.error = None;
        self.last_fetch_started = Some(now);
        vec![Effect::Refresh]
    }

    fn move_cursor(&mut self, by: isize) {
        let last = self.rows.len().saturating_sub(1) as isize;
        let at = (self.cursor() as isize + by).clamp(0, last.max(0));
        self.cursors[self.tab] = at as usize;
        if let (Some(s), Some(e)) = (&self.snapshot, self.selected()) {
            self.selected_ids[self.tab] = Some(EntryKey::of(s, e));
        }
        self.pane_scroll = 0;
    }

    pub fn item_metrics(&self) -> Vec<&worklens_core::raw::Metric> {
        let (Some(s), Some(e)) = (self.snapshot.as_ref(), self.selected()) else {
            return Vec::new();
        };
        let (Some(b), Some(report)) = (e.bug, s.raw.telemetry.as_ref()) else {
            return Vec::new();
        };
        let id = s.bug(b).id;
        report.metrics.iter().filter(|m| m.bugs.contains(&id)).collect()
    }

    pub fn pane_shown(&self, p: Pane) -> bool {
        match self.prefs.panes[p.index()] {
            _ if p == Pane::Overview => true,
            Show::Always => true,
            Show::Hidden => false,
            Show::Auto => self.pane_has_content(p),
        }
    }

    fn pane_has_content(&self, p: Pane) -> bool {
        let (Some(s), Some(e)) = (self.snapshot.as_ref(), self.selected()) else {
            return false;
        };
        match p {
            Pane::Overview => true,
            Pane::Timeline => timeline::events(s, e).len() >= 2 || timeline::ships(s, e).is_some(),
            Pane::Comments => e
                .bug
                .is_some_and(|b| s.raw.threads.get(&s.bug(b).id).is_none_or(|t| !t.comments.is_empty())),
            Pane::Links => !present::links(s, e).is_empty(),
            Pane::Drift => !e.drift.is_empty(),
            Pane::Telemetry => !self.item_metrics().is_empty(),
        }
    }

    pub fn current_pane(&self) -> Pane {
        if self.pane_shown(self.pane) { self.pane } else { Pane::Overview }
    }

    fn step_pane(&mut self, by: isize) {
        let mut p = self.current_pane();
        loop {
            p = p.step(by);
            if self.pane_shown(p) {
                break;
            }
        }
        self.pane = p;
        self.pane_scroll = 0;
    }

    fn switch_tab(&mut self, to: usize) {
        self.seen[self.tab] = self.seen_now();
        self.tab = if to == Section::Backlog.index() {
            Section::Other.index()
        } else {
            to % Section::COUNT
        };
        self.pane_scroll = 0;
        let before = std::mem::take(&mut self.seen[self.tab]);
        if let Some(gone) = self.rebuild_rows(&before) {
            let text = self.left_text(&gone, "");
            self.flash(text, false, self.now);
        }
    }

    pub fn tab_shown(&self, i: usize) -> bool {
        if i == Section::Backlog.index() {
            return false;
        }
        match self.prefs.sections[i] {
            _ if i == 0 => true,
            Show::Always => true,
            Show::Hidden => false,
            Show::Auto => i == self.tab || (self.snapshot.is_some() && self.counts[i] > 0),
        }
    }

    pub fn hidden_with_work(&self) -> Vec<usize> {
        if self.snapshot.is_none() {
            return Vec::new();
        }
        (0..Section::COUNT)
            .filter(|i| *i != Section::Backlog.index() && self.prefs.sections[*i] == Show::Hidden && self.counts[*i] > 0)
            .collect()
    }

    pub fn set_prefs(&mut self, prefs: Prefs) {
        self.show_hidden = prefs.show_hidden;
        self.prefs = prefs;
        self.refresh_rows();
    }

    pub fn current_setting(&self) -> Setting {
        let all = settings::all();
        all[self.setting.min(all.len() - 1)]
    }

    fn rebuild_snapshot(&mut self, now: Timestamp) {
        let Some(old) = self.snapshot.take() else { return };
        let mut s = build_snapshot(old.raw, &self.cfg, now);
        marks::apply(&mut s, &self.marks, now);
        self.snapshot = Some(s);
        self.refresh_rows();
    }

    fn change_setting(&mut self, setting: Setting, now: Timestamp) -> Vec<Effect> {
        match setting {
            Setting::Whiteboard => {
                self.cfg.whiteboard = Some(self.prefs.whiteboard.clone()).filter(|w| !w.is_empty());
            }
            Setting::SprintBudget => {
                self.cfg.sprint_budget = self.prefs.sprint_budget;
                self.rebuild_snapshot(now);
            }
            Setting::CacheMinutes => {
                if self.opts.cache_minutes.is_some() {
                    self.opts.cache_minutes = Some(self.prefs.cache_minutes);
                }
            }
            Setting::ShowHidden => {
                self.show_hidden = self.prefs.show_hidden;
                self.refresh_rows();
            }
            Setting::Section(i) if i == self.tab && !self.tab_shown(i) => self.switch_tab(0),
            _ => {}
        }
        let mut done = format!("{} {}", setting.label(), self.prefs.value(setting));
        let mut effects = Vec::new();
        if setting.refetches() {
            let refresh = self.begin_refresh(now);
            done.push_str(if refresh.is_empty() {
                "; applies at the next refresh"
            } else {
                "; refreshing"
            });
            effects.push(Effect::SaveSetting { setting, done });
            effects.extend(refresh);
        } else {
            effects.push(Effect::SaveSetting { setting, done });
        }
        effects
    }

    fn step_tab(&mut self, forward: bool) {
        let mut i = self.tab;
        loop {
            i = if forward {
                (i + 1) % Section::COUNT
            } else {
                (i + Section::COUNT - 1) % Section::COUNT
            };
            if self.tab_shown(i) {
                break;
            }
        }
        self.switch_tab(i);
    }

    fn touch(&mut self) {
        let (Some(s), Some(e)) = (self.snapshot.as_ref(), self.selected()) else {
            return;
        };
        let id = s.id(e).to_string();
        if self.touched.iter().any(|t| t.id == id) {
            return;
        }
        let touched = Touched {
            bug: e.bug.map(|b| s.bug(b).id),
            revisions: e.revisions.iter().chain(&e.review).map(|r| s.rev(*r).id).collect(),
            jira: e.issue.map(|i| s.issue(i).key.0.clone()),
            id,
        };
        self.touched.push(touched);
    }

    pub fn actions(&self) -> Vec<ItemAction> {
        let (Some(s), Some(e)) = (self.snapshot.as_ref(), self.selected()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(step) = s.next_step(e)
            && let Some(url) = worklens_core::guidance::next_url(&s.raw, step)
        {
            out.push(ItemAction {
                key: None,
                label: format!("Open next step: {step}"),
                kind: ActionKind::Open(url),
            });
        }
        let mut add = |key, label: String, kind| {
            out.push(ItemAction {
                key: Some(key),
                label,
                kind,
            })
        };
        let links = present::links(s, e);
        let first = |kind| links.iter().find(|l| l.kind == kind);
        if let (Some(b), Some(l)) = (e.bug.map(|b| s.bug(b)), first(LinkKind::Bug)) {
            add('B', format!("Open bug {} in Bugzilla", b.id), ActionKind::Open(l.url.clone()));
        }
        if let (Some(i), Some(l)) = (e.issue.map(|i| s.issue(i)), first(LinkKind::Jira)) {
            add('J', format!("Open {} in Jira", i.key), ActionKind::Open(l.url.clone()));
        }
        let idea = e.idea.map(|i| s.idea(i)).zip(first(LinkKind::Idea));
        if let (Some((i, l)), None) = (idea, e.issue) {
            add('J', format!("Open {} in Jira", i.key), ActionKind::Open(l.url.clone()));
        }
        let rev = s.revision_of(e).map(|r| s.rev(r));
        if let Some(r) = rev {
            add('P', format!("Open {} in Phabricator", r.id), ActionKind::Open(rev_url(r.id)));
        }
        if let Some(l) = first(LinkKind::Lando)
            && let Some(r) = l.rev
        {
            add('L', format!("Open {r} in Lando"), ActionKind::Open(l.url.clone()));
        }
        if let Some(l) = first(LinkKind::Try)
            && let Some(r) = l.rev
        {
            add('T', format!("Open {r}'s try push in Treeherder"), ActionKind::Open(l.url.clone()));
        }
        match e.id {
            ItemId::Idea(i) => add(
                'b',
                format!("Copy an engineering update for {}", s.idea(i).key),
                ActionKind::CopyBrief,
            ),
            _ => add('b', "Copy a brief for a coding agent".into(), ActionKind::CopyBrief),
        }
        if let Some(r) = rev {
            add(
                'C',
                format!("Check out {} locally (moz-phab patch)", r.id),
                ActionKind::Checkout(r.id),
            );
        }
        if let Some(r) = e.review.map(|r| s.rev(r)) {
            match &self.opts.agent {
                Some(a) => add(
                    'R',
                    format!("Review {} with {} (drafts feedback, never posts)", r.id, a.name),
                    ActionKind::Review,
                ),
                None => add(
                    'R',
                    format!("Copy a prompt for reviewing {} with an agent", r.id),
                    ActionKind::CopyReviewPrompt,
                ),
            }
        }
        if let Some(fix) = e.drift.iter().find_map(|d| d.fix.as_ref()) {
            let (what, url) = match fix.side {
                Side::Jira => (
                    e.issue.map(|i| s.issue(i).key.to_string()).unwrap_or_default(),
                    e.issue.map(|i| jira_url(&s.issue(i).key.0)),
                ),
                Side::Bugzilla => (
                    e.bug.map(|b| format!("bug {}", s.bug(b).id)).unwrap_or_default(),
                    e.bug.map(|b| bug_url(s.bug(b).id)),
                ),
            };
            let copied = if fix.value.starts_with("https://") {
                format!("the {} URL", fix.field)
            } else {
                format!("\"{}\"", fix.value)
            };
            add(
                'F',
                format!("Open {what} and copy {copied}"),
                ActionKind::FixDrift {
                    value: fix.value.clone(),
                    url,
                    field: fix.field,
                },
            );
        }
        let start = match &self.opts.agent {
            Some(a) => format!("Start {} with the brief", a.name),
            None => "Start a coding agent (none found; copies the brief)".into(),
        };
        add('c', start, ActionKind::StartAgent);
        add('y', format!("Copy \"{}\"", s.id(e)), ActionKind::CopyId);
        let done = if e.marks.hidden == Some(Hidden::Done) {
            "Bring back (undo done)"
        } else {
            "Done locally: hide until new activity"
        };
        add('d', done.into(), ActionKind::Done);
        let snooze = if e.marks.hidden == Some(Hidden::Snoozed) {
            "Bring back (undo snooze)"
        } else {
            "Snooze locally until a date or new activity…"
        };
        add('z', snooze.into(), ActionKind::Snooze);
        for dependency in worklens_core::dependencies::of(s, e) {
            if out
                .iter()
                .any(|action| matches!(&action.kind, ActionKind::Open(url) if *url == dependency.url))
            {
                continue;
            }
            let source = if dependency.source == "bugzilla" {
                "Bugzilla"
            } else {
                "Phabricator"
            };
            out.push(ItemAction {
                key: None,
                label: format!("Open dependency {} in {source}", dependency.id),
                kind: ActionKind::Open(dependency.url),
            });
        }
        for l in links.iter().filter(|l| matches!(l.kind, LinkKind::Parent | LinkKind::Idea)) {
            if out
                .iter()
                .any(|action| matches!(&action.kind, ActionKind::Open(url) if *url == l.url))
            {
                continue;
            }
            let key = l.url.rsplit('/').next().unwrap_or_default();
            out.push(ItemAction {
                key: None,
                label: format!("Open {} {key} in Jira", l.label.to_lowercase()),
                kind: ActionKind::Open(l.url.clone()),
            });
        }
        if let Some(report) = &s.raw.landings {
            for repo in report.repositories.iter().filter(|repo| repo.error.is_none()) {
                for landing in &repo.matches {
                    if !e.revisions.iter().chain(&e.review).any(|idx| s.rev(*idx).id == landing.revision)
                        || out
                            .iter()
                            .any(|action| matches!(&action.kind, ActionKind::Open(url) if *url == landing.url))
                    {
                        continue;
                    }
                    out.push(ItemAction {
                        key: None,
                        label: format!("Open {}'s {} push in Treeherder", landing.revision, repo.repository),
                        kind: ActionKind::Open(landing.url.clone()),
                    });
                }
            }
        }
        for (n, k) in self.opts.keys.iter().enumerate() {
            if !out.iter().any(|a| a.key == Some(k.key)) && !keys::RESERVED.contains(&k.key) && self.expand(&k.run, e).is_some() {
                out.push(ItemAction {
                    key: Some(k.key),
                    label: k.label.clone(),
                    kind: ActionKind::Custom(n),
                });
            }
        }
        out
    }

    pub fn custom_key_help(&self) -> Vec<(String, String)> {
        self.opts
            .keys
            .iter()
            .filter(|k| !keys::RESERVED.contains(&k.key))
            .map(|k| {
                let label = if keys::ITEM_LETTERS.contains(&k.key) {
                    format!("{} (where {} has no built-in use)", k.label, k.key)
                } else {
                    k.label.clone()
                };
                (k.key.to_string(), label)
            })
            .collect()
    }

    fn expand(&self, template: &str, e: &Entry) -> Option<String> {
        let s = self.snapshot.as_ref()?;
        let value = |name: &str| -> Option<String> {
            match name {
                "bug" => e.bug.map(|b| s.bug(b).id.to_string()),
                "rev" => s.revision_of(e).map(|r| s.rev(r).id.to_string()),
                "jira" => e.issue.map(|i| s.issue(i).key.to_string()),
                "id" => Some(s.id(e).to_string()),
                "dir" => Some(self.opts.dir.display().to_string()),
                _ => None,
            }
        };
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after.find("}}")?;
            out.push_str(&shell_quote(&value(&after[..end])?));
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        Some(out)
    }

    fn agent_command(&self, prompt: String, what: &str) -> Option<Command> {
        let a = self.opts.agent.as_ref()?;
        let mut args = a.args.clone();
        args.push(prompt);
        Some(Command {
            title: format!("{} in {} with {what}", a.name, self.opts.dir.display()),
            program: a.program.clone(),
            args,
            cwd: self.opts.dir.clone(),
        })
    }

    fn run_action(&mut self, action: ItemAction, now: Timestamp) -> Vec<Effect> {
        let cursor = self.cursor();
        let Some(s) = self.snapshot.as_ref() else { return Vec::new() };
        let Some(e) = selected_in(Some(s), &self.rows, cursor) else {
            return Vec::new();
        };
        let id = s.id(e).to_string();
        match action.kind {
            ActionKind::Open(url) => {
                self.touch();
                vec![Effect::Open(url)]
            }
            ActionKind::CopyBrief => vec![Effect::Copy {
                text: brief::render(s, e),
                done: if matches!(e.id, ItemId::Idea(_)) {
                    format!("Update for {id} copied; paste it into Jira or Slack")
                } else {
                    format!("Brief for {id} copied; paste it into your agent")
                },
            }],
            ActionKind::CopyId => vec![Effect::Copy {
                text: id.clone(),
                done: format!("Copied {id}"),
            }],
            ActionKind::CopyReviewPrompt => {
                let text = brief::review_prompt(s, e).unwrap_or_default();
                vec![Effect::Copy {
                    text,
                    done: format!("Review prompt for {id} copied"),
                }]
            }
            ActionKind::Checkout(rev) => {
                let command = Command {
                    title: format!("moz-phab patch {rev} in {}", self.opts.dir.display()),
                    program: "moz-phab".into(),
                    args: vec!["patch".into(), rev.to_string()],
                    cwd: self.opts.dir.clone(),
                };
                vec![Effect::RunInTerminal { command, pause: true }]
            }
            ActionKind::Review => {
                let rev = e.review.map(|r| s.rev(r).id.to_string()).unwrap_or_default();
                let prompt = brief::review_prompt(s, e).unwrap_or_default();
                self.agent_command(prompt, &format!("a review of {rev}"))
                    .map(|command| Effect::RunInTerminal { command, pause: false })
                    .into_iter()
                    .collect()
            }
            ActionKind::StartAgent => {
                let text = brief::render(s, e);
                match self.agent_command(text.clone(), &format!("the brief for {id}")) {
                    Some(command) => vec![Effect::RunInTerminal { command, pause: false }],
                    None => vec![Effect::Copy {
                        text,
                        done: format!("No claude or codex found (set \"agent\" in the config). Brief for {id} copied"),
                    }],
                }
            }
            ActionKind::FixDrift { value, url, field } => {
                self.touch();
                let mut out = vec![Effect::Copy {
                    text: value.clone(),
                    done: format!("Copied \"{value}\"; set {field} to it"),
                }];
                out.extend(url.map(Effect::Open));
                out
            }
            ActionKind::Done => {
                let back = e.marks.hidden == Some(Hidden::Done);
                let text = if back {
                    marks::restore(&mut self.marks, s, e);
                    format!("{id} is back")
                } else {
                    marks::mark_done(&mut self.marks, s, e, now);
                    format!(
                        "{id} done locally; returns on new activity{}",
                        if self.show_hidden { "" } else { " (Z shows it)" }
                    )
                };
                self.flash(text, false, now);
                self.marks_changed(now)
            }
            ActionKind::Snooze => {
                if e.marks.hidden == Some(Hidden::Snoozed) {
                    marks::restore(&mut self.marks, s, e);
                    self.flash(format!("{id} is back"), false, now);
                    return self.marks_changed(now);
                }
                self.snooze_text.clear();
                self.snooze_for = Some(EntryKey::of(s, e));
                self.mode = Mode::Snooze;
                Vec::new()
            }
            ActionKind::Custom(n) => {
                let k = &self.opts.keys[n];
                let Some(cmd) = self.expand(&k.run, e) else { return Vec::new() };
                let command = Command {
                    title: k.label.clone(),
                    program: "sh".into(),
                    args: vec!["-c".into(), cmd],
                    cwd: self.opts.dir.clone(),
                };
                if k.terminal {
                    vec![Effect::RunInTerminal { command, pause: true }]
                } else {
                    vec![Effect::RunQuietly {
                        command,
                        label: k.label.clone(),
                    }]
                }
            }
        }
    }

    fn snooze(&mut self, now: Timestamp, zoned_now: &jiff::Zoned) -> Vec<Effect> {
        let text = self.snooze_text.trim().to_owned();
        let until = if text.is_empty() {
            None
        } else {
            marks::parse_when(&text, zoned_now)
        };
        if !text.is_empty() && until.is_none() {
            self.flash(format!("Can't read \"{text}\"; try tomorrow, mon, 3d or nov 10"), true, now);
            return Vec::new();
        }
        let cursor = self.cursor();
        let target = selected_in(self.snapshot.as_ref(), &self.rows, cursor)
            .zip(self.snapshot.as_ref())
            .filter(|(e, s)| self.snooze_for.as_ref().is_some_and(|k| k.is(s, e)));
        let Some((e, s)) = target else {
            self.mode = Mode::List;
            self.flash("That item left this section in the refresh; nothing snoozed", true, now);
            return Vec::new();
        };
        let id = s.id(e).to_string();
        marks::mark_snoozed(&mut self.marks, s, e, until, now);
        self.mode = Mode::List;
        let when = until.map_or_else(|| "it has new activity".to_owned(), |t| present::date(t, zoned_now.time_zone()));
        self.flash(format!("{id} snoozed locally until {when}; Z shows hidden"), false, now);
        self.marks_changed(now)
    }

    fn quit(&mut self, now: Timestamp) -> Vec<Effect> {
        self.marks.last_look = Some(now);
        vec![Effect::SaveMarks, Effect::Quit]
    }
}

struct Gone {
    key: EntryKey,
    neighbor: bool,
}

fn selected_in<'a>(snapshot: Option<&'a Snapshot>, rows: &[EntryIdx], cursor: usize) -> Option<&'a Entry> {
    let s = snapshot?;
    rows.get(cursor).map(|i| s.entry(*i))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn minutes_between(a: Timestamp, b: Timestamp) -> f64 {
    (b.as_millisecond() - a.as_millisecond()) as f64 / 60_000.0
}

pub fn update(app: &mut App, msg: Msg, now: Timestamp) -> Vec<Effect> {
    app.dirty = true;
    app.now = now;
    match msg {
        Msg::Key(key) => {
            app.pressed = true;
            app.since = None;
            on_key(app, key, now)
        }
        Msg::Resize(w, h) => {
            app.size = (w, h);
            Vec::new()
        }
        Msg::Focus(true) if !app.touched.is_empty() && app.busy.is_none() => {
            let items = std::mem::take(&mut app.touched);
            let ids: Vec<&str> = items.iter().map(|t| t.id.as_str()).collect();
            app.busy = Some(format!("Checking {}", ids.join(", ")));
            vec![Effect::Refetch(items)]
        }
        Msg::Focus(_) => {
            app.dirty = false;
            Vec::new()
        }
        Msg::Progress(text) => {
            app.busy = Some(text);
            Vec::new()
        }
        Msg::Fetched(result) => {
            app.busy = None;
            match result {
                Ok(raw) => {
                    app.set_raw(*raw, now);
                    if app.mode == Mode::Setup && app.page == Page::Sources {
                        app.mode = Mode::List;
                    }
                    if let Some((mine, team)) = app.snapshot.as_ref().and_then(worklens_core::drift_log::counts) {
                        let date = now.to_zoned(jiff::tz::TimeZone::system()).date();
                        app.drift_log.record(date, mine, team);
                        return vec![Effect::RecordDrift(worklens_core::drift_log::DriftDay { date, mine, team })];
                    }
                }
                Err(e) => {
                    app.error = Some(e);
                    if app.snapshot.is_none() && app.mode != Mode::Setup {
                        return open_setup(app);
                    }
                }
            }
            Vec::new()
        }
        Msg::Done(result) => {
            match result {
                Ok(text) => {
                    app.action_error = None;
                    app.flash(text, false, now);
                }
                Err(text) => {
                    app.action_error = Some(text.clone());
                    app.flash(text, true, now);
                }
            }
            Vec::new()
        }
        Msg::Checks(checks) => {
            app.checks = Some(checks);
            Vec::new()
        }
        Msg::Tick => on_tick(app, now),
    }
}

fn open_setup(app: &mut App) -> Vec<Effect> {
    app.pane_scroll = 0;
    app.page = Page::Sources;
    app.editing = None;
    app.mode = Mode::Setup;
    app.checks = None;
    vec![Effect::RunChecks]
}

fn on_tick(app: &mut App, now: Timestamp) -> Vec<Effect> {
    let mut changed = false;
    if app.busy.is_some() || app.checks_pending() {
        app.frame = app.frame.wrapping_add(1);
        changed = true;
    }
    if app.toast.as_ref().is_some_and(|t| t.until <= now) {
        app.toast = None;
        changed = true;
    }
    let mut effects = Vec::new();
    let selected = app.snapshot.as_ref().zip(app.selected()).map(|(s, e)| EntryKey::of(s, e));
    match (&app.resting, selected) {
        (Some((id, since)), Some(sel)) if *id == sel => {
            if (now.as_millisecond() - since.as_millisecond()) >= SEEN_AFTER_MS {
                let cursor = app.cursor();
                let Some(s) = app.snapshot.as_ref() else { return effects };
                let Some(e) = selected_in(Some(s), &app.rows, cursor) else {
                    return effects;
                };
                if e.marks.is_new
                    || app
                        .marks
                        .items
                        .get(&worklens_core::key::mark_key(s, e))
                        .is_none_or(|m| m.seen.is_none())
                {
                    let kept = (app.rows[cursor], e.changes.clone());
                    marks::mark_seen(&mut app.marks, s, e, now);
                    effects.extend(app.marks_changed(now));
                    if let Some(s) = app.snapshot.as_mut() {
                        s.entries[kept.0.get()].changes = kept.1;
                    }
                    changed = true;
                }
                app.resting = Some((sel, Timestamp::MAX));
            }
        }
        (_, Some(sel)) => app.resting = Some((sel, now)),
        (_, None) => app.resting = None,
    }
    if let (Some(max), Some(started)) = (app.opts.cache_minutes, app.last_fetch_started)
        && minutes_between(started, now) >= max
    {
        effects.extend(app.begin_refresh(now));
    }
    app.dirty = changed || !effects.is_empty();
    effects
}

fn on_key(app: &mut App, key: KeyEvent, now: Timestamp) -> Vec<Effect> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return app.quit(now);
    }
    match app.mode {
        Mode::Filter => {
            match key.code {
                KeyCode::Esc => {
                    app.filter.clear();
                    app.mode = Mode::List;
                }
                KeyCode::Enter => app.mode = Mode::List,
                KeyCode::Backspace => {
                    app.filter.pop();
                }
                KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => app.filter.push(c),
                _ => {}
            }
            app.refresh_rows();
            Vec::new()
        }
        Mode::Snooze => match key.code {
            KeyCode::Esc => {
                app.mode = Mode::List;
                Vec::new()
            }
            KeyCode::Backspace => {
                app.snooze_text.pop();
                Vec::new()
            }
            KeyCode::Enter => app.snooze(now, &now.to_zoned(jiff::tz::TimeZone::system())),
            KeyCode::Char(c) => {
                app.snooze_text.push(c);
                Vec::new()
            }
            _ => Vec::new(),
        },
        Mode::Log => {
            let max = app.snapshot.as_ref().map_or(0, |s| s.raw.calls.len().saturating_sub(1));
            match key.code {
                KeyCode::Esc | KeyCode::Char('@') => app.mode = Mode::List,
                KeyCode::Char('q') => return app.quit(now),
                KeyCode::Down | KeyCode::Char('j') => app.log_offset = (app.log_offset + 1).min(max),
                KeyCode::Up | KeyCode::Char('k') => app.log_offset = app.log_offset.saturating_sub(1),
                KeyCode::PageDown => app.log_offset = (app.log_offset + 10).min(max),
                KeyCode::PageUp => app.log_offset = app.log_offset.saturating_sub(10),
                _ => {}
            }
            Vec::new()
        }
        Mode::Help => {
            let max = crate::view::help_scroll_max(app);
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => app.mode = Mode::List,
                KeyCode::Char('q') => return app.quit(now),
                KeyCode::Down | KeyCode::Char('j') => app.help_scroll = (app.help_scroll + 1).min(max),
                KeyCode::Up | KeyCode::Char('k') => app.help_scroll = app.help_scroll.saturating_sub(1),
                KeyCode::PageDown => app.help_scroll = app.help_scroll.saturating_add(10).min(max),
                KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(10),
                _ => {}
            }
            Vec::new()
        }
        Mode::Setup if app.editing.is_some() => on_edit_key(app, key, now),
        Mode::Setup if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) => {
            app.page = match app.page {
                Page::Sources => Page::Settings,
                Page::Settings => Page::Sources,
            };
            app.pane_scroll = 0;
            app.setting = 0;
            Vec::new()
        }
        Mode::Setup if app.page == Page::Settings => on_settings_key(app, key, now),
        Mode::Setup => match key.code {
            KeyCode::PageDown => {
                app.pane_scroll = app.pane_scroll.saturating_add(10).min(crate::view::sources_scroll_max(app));
                Vec::new()
            }
            KeyCode::PageUp => {
                app.pane_scroll = app.pane_scroll.saturating_sub(10);
                Vec::new()
            }
            KeyCode::Char('r') => app.begin_refresh(now),
            KeyCode::Char('q') => app.quit(now),
            KeyCode::Esc if app.snapshot.is_some() => {
                app.mode = Mode::List;
                Vec::new()
            }
            KeyCode::Esc => app.quit(now),
            _ => Vec::new(),
        },
        Mode::Menu => {
            let mut actions = app.actions();
            let pick = match key.code {
                KeyCode::PageDown => {
                    app.menu_scroll = app.menu_scroll.saturating_add(5).min(crate::view::menu_scroll_max(app, &actions));
                    return Vec::new();
                }
                KeyCode::PageUp => {
                    app.menu_scroll = app.menu_scroll.saturating_sub(5);
                    return Vec::new();
                }
                KeyCode::Esc => {
                    app.mode = Mode::List;
                    return Vec::new();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.menu_scroll = 0;
                    app.menu_cursor = app.menu_cursor.saturating_sub(1);
                    return Vec::new();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.menu_scroll = 0;
                    app.menu_cursor = (app.menu_cursor + 1).min(actions.len().saturating_sub(1));
                    return Vec::new();
                }
                KeyCode::Enter => Some(app.menu_cursor),
                KeyCode::Char(c @ '1'..='9') => Some(c as usize - '1' as usize),
                KeyCode::Char('0') => Some(9),
                KeyCode::Char(c) => actions.iter().position(|a| a.key == Some(c)),
                _ => None,
            };
            match pick.filter(|n| *n < actions.len()) {
                Some(n) => {
                    app.mode = Mode::List;
                    let action = actions.swap_remove(n);
                    app.run_action(action, now)
                }
                None => Vec::new(),
            }
        }
        Mode::List => on_list_key(app, key, now),
    }
}

fn on_edit_key(app: &mut App, key: KeyEvent, now: Timestamp) -> Vec<Effect> {
    let setting = app.current_setting();
    let Some(text) = app.editing.as_mut() else { return Vec::new() };
    match key.code {
        KeyCode::Esc => app.editing = None,
        KeyCode::Backspace => {
            text.pop();
        }
        KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => text.push(c),
        KeyCode::Enter => {
            let text = text.clone();
            let before = app.prefs.text(setting);
            return match app.prefs.set_text(setting, &text) {
                Err(e) => {
                    app.flash(e, true, now);
                    Vec::new()
                }
                Ok(()) => {
                    app.editing = None;
                    if app.prefs.text(setting) == before {
                        Vec::new()
                    } else {
                        app.change_setting(setting, now)
                    }
                }
            };
        }
        _ => {}
    }
    Vec::new()
}

fn on_settings_key(app: &mut App, key: KeyEvent, now: Timestamp) -> Vec<Effect> {
    let last = settings::all().len() - 1;
    let effects = settings_key(app, key, now, last);
    let line = settings::line_of(app.setting) + if app.setting == last { 2 } else { 0 };
    let height = app.size.1.saturating_sub(6).max(1) as usize;
    let top = app.pane_scroll as usize;
    let top = if line <= top {
        line.saturating_sub(1)
    } else if line >= top + height {
        line + 1 - height
    } else {
        top
    };
    app.pane_scroll = top.min(u16::MAX as usize) as u16;
    effects
}

fn settings_key(app: &mut App, key: KeyEvent, now: Timestamp, last: usize) -> Vec<Effect> {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => app.setting = (app.setting + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => app.setting = app.setting.saturating_sub(1),
        KeyCode::PageDown => app.setting = (app.setting + 10).min(last),
        KeyCode::PageUp => app.setting = app.setting.saturating_sub(10),
        KeyCode::Home => app.setting = 0,
        KeyCode::End => app.setting = last,
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Left | KeyCode::Char('l' | 'h') => {
            let setting = app.current_setting();
            let by = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                -1
            } else {
                1
            };
            let arrow = !matches!(key.code, KeyCode::Enter | KeyCode::Char(' '));
            match setting.kind() {
                Kind::Fixed => app.flash(format!("{} is always shown", setting.label()), false, now),
                Kind::Text if arrow => {}
                Kind::Text => app.editing = Some(app.prefs.text(setting)),
                Kind::Toggle => {
                    app.prefs.step(setting, by);
                    return app.change_setting(setting, now);
                }
            }
        }
        KeyCode::Char('r') => return app.begin_refresh(now),
        KeyCode::Char('q') => return app.quit(now),
        KeyCode::Esc if app.snapshot.is_some() => app.mode = Mode::List,
        KeyCode::Esc => return app.quit(now),
        _ => {}
    }
    Vec::new()
}

fn on_list_key(app: &mut App, key: KeyEvent, now: Timestamp) -> Vec<Effect> {
    let Some(action) = keys::action_for(key) else {
        let KeyCode::Char(c) = key.code else { return Vec::new() };
        let mut actions = app.actions();
        return match actions.iter().position(|a| a.key == Some(c)) {
            Some(n) => {
                let action = actions.swap_remove(n);
                app.run_action(action, now)
            }
            None => {
                app.dirty = false;
                Vec::new()
            }
        };
    };
    let (detail_page, detail_max) = crate::view::detail_paging(app);
    match action {
        Action::Quit => return app.quit(now),
        Action::Back if !app.filter.is_empty() => {
            app.filter.clear();
            app.refresh_rows();
        }
        Action::Back => return app.quit(now),
        Action::Help => {
            app.mode = Mode::Help;
            app.help_scroll = 0;
        }
        Action::Setup => return open_setup(app),
        Action::Log => {
            app.log_offset = 0;
            app.mode = Mode::Log;
        }
        Action::Down => app.move_cursor(1),
        Action::Up => app.move_cursor(-1),
        Action::PageDown => app.pane_scroll = app.pane_scroll.min(detail_max).saturating_add(detail_page).min(detail_max),
        Action::PageUp => app.pane_scroll = app.pane_scroll.min(detail_max).saturating_sub(detail_page),
        Action::Top => app.move_cursor(isize::MIN / 2),
        Action::Bottom => app.move_cursor(isize::MAX / 2),
        Action::NextSection => app.step_tab(true),
        Action::PrevSection => app.step_tab(false),
        Action::Section(n) if app.prefs.sections.get(n).is_some_and(|s| *s != Show::Hidden) => app.switch_tab(n),
        Action::Section(_) => {}
        Action::NextPane => app.step_pane(1),
        Action::PrevPane => app.step_pane(-1),
        Action::Filter => app.mode = Mode::Filter,
        Action::ToggleHidden => {
            app.show_hidden = !app.show_hidden;
            app.refresh_rows();
            let text = if app.show_hidden {
                "Showing done and snoozed items (Z hides them)"
            } else {
                "Hiding done and snoozed items"
            };
            app.flash(text, false, now);
        }
        Action::Refresh => return app.begin_refresh(now),
        Action::Menu if app.selected().is_some() => {
            app.menu_cursor = 0;
            app.menu_scroll = 0;
            app.mode = Mode::Menu;
        }
        Action::Open => {
            let url = app.snapshot.as_ref().zip(app.selected()).and_then(|(s, e)| {
                s.next_step(e)
                    .and_then(|step| worklens_core::guidance::next_url(&s.raw, step))
                    .or_else(|| {
                        e.bug
                            .map(|b| bug_url(s.bug(b).id))
                            .or_else(|| e.issue.map(|i| jira_url(&s.issue(i).key.0)))
                            .or_else(|| s.revision_of(e).map(|r| rev_url(s.rev(r).id)))
                    })
            });
            if let Some(url) = url {
                app.touch();
                return vec![Effect::Open(url)];
            }
        }
        Action::Menu => {}
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn code(c: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn now() -> Timestamp {
        "2026-11-02T12:00:00Z".parse().unwrap()
    }

    fn fixture() -> Raw {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn app() -> App {
        let raw = fixture();
        let opts = Options {
            cache_minutes: None,
            agent: None,
            dir: PathBuf::from("/tmp"),
            keys: Vec::new(),
        };
        let mut app = App::new(CoreConfig::default(), opts, Some(raw), Marks::default(), now());
        app.size = (80, 24);
        app
    }

    #[test]
    fn arrows_skip_empty_sections_and_wrap() {
        let mut a = app();
        assert_eq!(a.section(), Section::Waiting);
        let with_work: Vec<Section> = (1..Section::COUNT).filter(|i| a.counts[*i] > 0).map(|i| Section::ALL[i]).collect();
        assert!(!with_work.is_empty(), "the fixture needs a non-empty section: {:?}", a.counts);
        update(&mut a, code(KeyCode::Right), now());
        assert_eq!(a.section(), with_work[0], "right skips empty sections");
        for _ in 1..with_work.len() {
            update(&mut a, code(KeyCode::Right), now());
        }
        update(&mut a, code(KeyCode::Right), now());
        assert_eq!(a.section(), Section::Waiting, "right from the last with work wraps to the first");
        update(&mut a, code(KeyCode::Left), now());
        assert_eq!(
            a.section(),
            *with_work.last().unwrap(),
            "left from the first wraps to the last with work"
        );
    }

    #[test]
    fn a_digit_reaches_an_empty_section_and_it_stays_shown() {
        let mut a = app();
        let empty = (1..Section::COUNT)
            .find(|i| a.counts[*i] == 0)
            .expect("the fixture has an empty section");
        assert!(!a.tab_shown(empty));
        update(&mut a, key(char::from_digit(empty as u32 + 1, 10).unwrap()), now());
        assert_eq!(a.tab, empty);
        assert!(a.tab_shown(empty), "the current section is shown even when empty");
        assert!(a.tab_shown(0));
        update(&mut a, key('1'), now());
        assert_eq!(a.section(), Section::Waiting);
    }

    #[test]
    fn done_hides_the_item_and_saves_marks() {
        let mut a = app();
        assert_eq!(a.rows.len(), 1);
        let effects = update(&mut a, key('d'), now());
        assert!(matches!(effects.as_slice(), [Effect::SaveMarks]));
        assert!(a.rows.is_empty());
        assert!(a.toast.as_ref().unwrap().text.contains("done"));
        update(&mut a, key('Z'), now());
        assert_eq!(a.rows.len(), 1, "Z shows done items");
        update(&mut a, key('d'), now());
        update(&mut a, key('Z'), now());
        assert_eq!(a.rows.len(), 1, "d on a done item brings it back");
    }

    #[test]
    fn snooze_asks_until_when_and_reads_the_answer() {
        let mut a = app();
        update(&mut a, key('z'), now());
        assert_eq!(a.mode, Mode::Snooze);
        for c in "someday".chars() {
            update(&mut a, key(c), now());
        }
        update(&mut a, code(KeyCode::Enter), now());
        assert!(a.toast.as_ref().unwrap().error, "unreadable dates are refused");
        a.snooze_text = "3d".into();
        update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(a.mode, Mode::List);
        assert!(a.rows.is_empty());
    }

    #[test]
    fn the_menu_lists_actions_and_runs_one_by_number() {
        let mut a = app();
        update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(a.mode, Mode::Menu);
        let labels: Vec<String> = a
            .actions()
            .iter()
            .map(|x| format!("{} {}", x.key.unwrap_or(' '), x.label))
            .collect();
        assert_eq!(labels[0], "B Open bug 1995008 in Bugzilla");
        assert!(labels.iter().any(|l| l.starts_with("P Open D321987")), "{labels:?}");
        assert!(labels.iter().any(|l| l.starts_with("T Open D321987's try push")), "{labels:?}");
        let effects = update(&mut a, key('1'), now());
        assert!(matches!(&effects[..], [Effect::Open(url)] if url.ends_with("id=1995008")));
        assert_eq!(a.mode, Mode::List);
    }

    #[test]
    fn integration_push_is_a_menu_action_without_a_new_shortcut() {
        let mut a = app();
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.landings = Some(
            serde_json::from_value(serde_json::json!({
                "revisions":[321987],"repositories":[{
                    "repository":"autoland","checkedAt":"2026-11-03T19:00:00Z",
                    "pushLimit":100,"scannedPushes":1,"oldestPush":"2026-11-03T10:00:00Z","error":null,
                    "matches":[{"revision":321987,"commit":"1111111111111111111111111111111111111111",
                        "pushRevision":"2222222222222222222222222222222222222222",
                        "url":"https://treeherder.mozilla.org/jobs?repo=autoland&revision=2222222222222222222222222222222222222222",
                        "pushedAt":"2026-11-03T10:00:00Z","health":null,"healthError":null}]
                }]
            }))
            .unwrap(),
        );
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        let actions = a.actions();
        let at = actions.iter().position(|action| action.label.contains("autoland push")).unwrap();
        assert_eq!(actions[at].key, None);
        update(&mut a, code(KeyCode::Enter), now());
        for _ in 0..at {
            update(&mut a, code(KeyCode::Down), now());
        }
        let effects = update(&mut a, code(KeyCode::Enter), now());
        assert!(
            matches!(&effects[..], [Effect::Open(url)] if url.contains("repo=autoland&revision=2222222222222222222222222222222222222222"))
        );
    }

    #[test]
    fn opening_an_item_rechecks_it_when_the_terminal_regains_focus() {
        let mut a = app();
        update(&mut a, key('B'), now());
        assert!(update(&mut a, Msg::Focus(false), now()).is_empty());
        let effects = update(&mut a, Msg::Focus(true), now());
        assert!(matches!(&effects[..], [Effect::Refetch(items)] if items[0].bug == Some(BugId(1_995_008))));
        assert!(a.busy.as_deref().unwrap().starts_with("Checking 1995008"));
        assert!(update(&mut a, Msg::Focus(true), now()).is_empty(), "only once");
    }

    #[test]
    fn filter_narrows_every_section_and_esc_clears_it() {
        let mut a = app();
        update(&mut a, key('/'), now());
        for c in "nomatch".chars() {
            update(&mut a, key(c), now());
        }
        assert_eq!(a.counts.iter().sum::<usize>(), 0);
        update(&mut a, code(KeyCode::Esc), now());
        assert_eq!(a.rows.len(), 1);
    }

    #[test]
    fn custom_keys_fill_in_ids_shell_quoted() {
        let mut a = app();
        a.opts.keys.push(CustomKey {
            key: 'S',
            label: "Searchfox".into(),
            run: "open https://searchfox.org/?q={{bug}} {{jira}}".into(),
            terminal: false,
        });
        let effects = update(&mut a, key('S'), now());
        let [Effect::RunQuietly { command, .. }] = &effects[..] else {
            panic!("{effects:?}")
        };
        assert_eq!(command.args[1], "open https://searchfox.org/?q='1995008' 'AIPLAT-1416'");
    }

    #[test]
    fn s_opens_setup_and_runs_the_checks() {
        let mut a = app();
        let effects = update(&mut a, key('s'), now());
        assert!(matches!(&effects[..], [Effect::RunChecks]));
        assert_eq!(a.mode, Mode::Setup);
        assert!(a.checks.is_none(), "checking…");
        update(
            &mut a,
            Msg::Checks(vec![Check {
                name: "jira".into(),
                ok: false,
                required: false,
                detail: "no token".into(),
                fix: Some("run jira-aiplat".into()),
                pending: false,
            }]),
            now(),
        );
        assert_eq!(a.checks.as_ref().map(Vec::len), Some(1));
    }

    #[test]
    fn q_quits_and_records_the_last_look() {
        let mut a = app();
        let effects = update(&mut a, key('q'), now());
        assert!(matches!(&effects[..], [Effect::SaveMarks, Effect::Quit]));
        assert_eq!(a.marks.last_look, Some(now()));
    }
    #[test]
    fn refresh_preserves_the_target_and_closes_an_old_menu() {
        let mut a = app();
        let mut raw = a.snapshot.take().unwrap().raw;
        let mut earlier = app().snapshot.take().unwrap().raw.bugs.remove(0);
        earlier.id = BugId(2_000_099);
        earlier.flags[0].creation_date = "2026-10-01T00:00:00Z".parse().unwrap();
        raw.bugs.insert(0, earlier);
        update(&mut a, code(KeyCode::Enter), now());
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        let s = a.snapshot.as_ref().unwrap();
        assert_eq!(s.id(a.selected().unwrap()).to_string(), "1995008");
        update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(a.mode, Mode::Menu);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.bugs.reverse();
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        assert_eq!(a.mode, Mode::List);
        assert!(a.toast.as_ref().unwrap().text.contains("reopens actions"));
        let s = a.snapshot.as_ref().unwrap();
        assert_eq!(s.id(a.selected().unwrap()).to_string(), "1995008");
        update(&mut a, code(KeyCode::Enter), now());
        let effects = update(&mut a, key('B'), now());
        assert!(matches!(&effects[..], [Effect::Open(url)] if url.ends_with("1995008")));
    }

    #[test]
    fn a_refresh_keeps_the_snooze_prompt_and_what_was_typed() {
        let mut a = app();
        update(&mut a, key('z'), now());
        for c in "3d".chars() {
            update(&mut a, key(c), now());
        }
        let raw = a.snapshot.take().unwrap().raw;
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        assert_eq!(a.mode, Mode::Snooze);
        assert_eq!(a.snooze_text, "3d");
        update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(a.mode, Mode::List);
        assert!(a.rows.is_empty(), "the same item is snoozed");
    }

    #[test]
    fn a_snooze_whose_item_left_in_a_refresh_is_canceled() {
        let mut a = app();
        fetched(&mut a, raw_with(&[2_000_099]));
        update(&mut a, key('z'), now());
        for c in "3d".chars() {
            update(&mut a, key(c), now());
        }
        let mut raw = raw_with(&[2_000_099]);
        raw.bugs.retain(|b| b.id != BugId(1_995_008));
        fetched(&mut a, raw);
        assert_eq!(a.mode, Mode::List);
        assert!(a.toast.as_ref().unwrap().error);
        assert_eq!(toast(&a), "1995008 is no longer in your work; snooze canceled; selected 2000099");
        assert!(a.marks.items.values().all(|m| m.snooze.is_none()), "nothing snoozed");
    }

    #[test]
    fn left_in_the_menu_does_not_close_it() {
        let mut a = app();
        update(&mut a, code(KeyCode::Enter), now());
        update(&mut a, code(KeyCode::Left), now());
        assert_eq!(a.mode, Mode::Menu);
        update(&mut a, code(KeyCode::Esc), now());
        assert_eq!(a.mode, Mode::List);
    }

    #[test]
    fn every_action_letter_is_described_in_help() {
        let mut a = app();
        a.opts.agent = Some(Agent {
            name: "claude".into(),
            program: "claude".into(),
            args: Vec::new(),
        });
        a.opts.keys.push(CustomKey {
            key: 'S',
            label: "Searchfox".into(),
            run: "echo {{bug}}".into(),
            terminal: false,
        });
        let help = a.custom_key_help();
        assert_eq!(help, vec![("S".to_owned(), "Searchfox".to_owned())]);
        for tab in 0..Section::COUNT {
            a.switch_tab(tab);
            for row in 0..a.rows.len() {
                a.cursors[tab] = row;
                for action in a.actions() {
                    let Some(c) = action.key else { continue };
                    assert!(
                        keys::ITEM_LETTERS.contains(&c) || help.iter().any(|(k, _)| k.contains(c)),
                        "{c} ({}) isn't in help",
                        action.label
                    );
                }
            }
        }
    }

    fn raw_with(ids: &[u32]) -> Raw {
        let mut raw = fixture();
        for id in ids {
            let mut copy = fixture().bugs.remove(0);
            copy.id = BugId(*id);
            raw.bugs.push(copy);
        }
        raw
    }

    fn row_ids(a: &App) -> Vec<String> {
        let s = a.snapshot.as_ref().unwrap();
        a.rows.iter().map(|i| s.id(s.entry(*i)).to_string()).collect()
    }

    fn selected_id(a: &App) -> Option<String> {
        a.selected().map(|e| a.snapshot.as_ref().unwrap().id(e).to_string())
    }

    fn fetched(a: &mut App, raw: Raw) {
        update(a, Msg::Fetched(Ok(Box::new(raw))), now());
    }

    const COPIES: [u32; 3] = [2_000_011, 2_000_012, 2_000_013];

    #[test]
    fn when_the_selected_item_leaves_the_next_neighbor_is_selected() {
        let mut a = app();
        fetched(&mut a, raw_with(&COPIES));
        let before = row_ids(&a);
        assert_eq!(before.len(), 4);
        update(&mut a, code(KeyCode::Down), now());
        update(&mut a, code(KeyCode::Down), now());
        let gone = selected_id(&a).unwrap();
        let mut raw = raw_with(&COPIES);
        raw.bugs.retain(|b| b.id.to_string() != gone);
        let mut earlier = fixture().bugs.remove(0);
        earlier.id = BugId(2_000_099);
        earlier.flags[0].creation_date = "2026-10-01T00:00:00Z".parse().unwrap();
        raw.bugs.push(earlier);
        fetched(&mut a, raw);
        assert_eq!(row_ids(&a)[0], "2000099", "the new item sorts first");
        assert_eq!(selected_id(&a).as_deref(), Some(before[3].as_str()), "the next neighbor");
        assert_eq!(toast(&a), format!("{gone} is no longer in your work; selected {}", before[3]));
    }

    #[test]
    fn when_the_last_item_moves_away_the_previous_is_selected_and_its_section_named() {
        let mut a = app();
        fetched(&mut a, raw_with(&COPIES));
        let before = row_ids(&a);
        update(&mut a, code(KeyCode::End), now());
        let gone = selected_id(&a).unwrap();
        let mut raw = raw_with(&COPIES);
        raw.bugs
            .iter_mut()
            .filter(|b| b.id.to_string() == gone)
            .for_each(|b| b.flags.clear());
        fetched(&mut a, raw);
        assert_eq!(selected_id(&a).as_deref(), Some(before[2].as_str()), "the previous neighbor");
        assert_eq!(toast(&a), format!("{gone} is in Sprint now; selected {}", before[2]));
    }

    #[test]
    fn a_refresh_that_removes_the_menus_item_closes_it_and_says_so() {
        let mut a = app();
        update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(a.mode, Mode::Menu);
        let mut raw = fixture();
        raw.bugs.clear();
        fetched(&mut a, raw);
        assert_eq!(a.mode, Mode::List);
        assert!(a.selected().is_none());
        assert_eq!(toast(&a), "1995008 is no longer in your work; actions closed; nothing else here");
        let effects = update(&mut a, key('B'), now());
        assert!(effects.is_empty(), "no action runs on another item: {effects:?}");
    }

    #[test]
    fn an_item_the_filter_stops_matching_is_not_said_to_have_left() {
        let mut a = app();
        let titled = |renamed: Option<u32>| {
            let mut raw = raw_with(&COPIES);
            for b in raw.bugs.iter_mut().skip(1) {
                b.summary = if Some(b.id.0) == renamed { "beta" } else { "alpha" }.into();
            }
            raw
        };
        fetched(&mut a, titled(None));
        update(&mut a, key('/'), now());
        for c in "alpha".chars() {
            update(&mut a, key(c), now());
        }
        update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(row_ids(&a).len(), 3);
        let gone = selected_id(&a).unwrap();
        fetched(&mut a, titled(Some(gone.parse().unwrap())));
        let toast = toast(&a);
        assert!(
            toast.starts_with(&format!("{gone} no longer matches the filter; selected ")),
            "{toast}"
        );
    }

    fn toast(a: &App) -> &str {
        let text = &a.toast.as_ref().unwrap().text;
        assert!(text.chars().count() <= 71, "too long for 80 columns: {text}");
        text
    }

    fn without(gone: &str) -> Raw {
        let mut raw = raw_with(&COPIES);
        raw.bugs.retain(|b| b.id.to_string() != gone);
        let mut earlier = fixture().bugs.remove(0);
        earlier.id = BugId(2_000_099);
        earlier.flags[0].creation_date = "2026-10-01T00:00:00Z".parse().unwrap();
        raw.bugs.push(earlier);
        raw
    }

    #[test]
    fn coming_back_to_a_section_whose_item_left_selects_its_neighbor() {
        let mut a = app();
        fetched(&mut a, raw_with(&COPIES));
        let before = row_ids(&a);
        update(&mut a, code(KeyCode::Down), now());
        update(&mut a, code(KeyCode::Down), now());
        let gone = selected_id(&a).unwrap();
        update(&mut a, code(KeyCode::Right), now());
        assert_ne!(a.section(), Section::Waiting);
        fetched(&mut a, without(&gone));
        update(&mut a, code(KeyCode::Left), now());
        assert_eq!(a.section(), Section::Waiting);
        assert_eq!(selected_id(&a).as_deref(), Some(before[3].as_str()), "the next neighbor");
        assert_eq!(toast(&a), format!("{gone} is no longer in your work; selected {}", before[3]));
    }

    #[test]
    fn when_only_new_work_is_left_the_toast_says_the_selection_is_new() {
        let mut a = app();
        let mut raw = raw_with(&[2_000_099]);
        raw.bugs.retain(|b| b.id != BugId(1_995_008));
        fetched(&mut a, raw);
        assert_eq!(toast(&a), "1995008 is no longer in your work; selected 2000099, new here");
    }

    #[test]
    fn a_reorder_under_an_open_menu_keeps_the_item_and_its_actions() {
        let dated = |order: [u32; 4]| {
            let mut raw = raw_with(&COPIES);
            for b in &mut raw.bugs {
                let Some(at) = order.iter().position(|id| *id == b.id.0) else {
                    continue;
                };
                let day = at + 1;
                b.flags[0].creation_date = format!("2026-10-0{day}T00:00:00Z").parse().unwrap();
            }
            raw
        };
        let mut a = app();
        fetched(&mut a, dated([1_995_008, 2_000_011, 2_000_012, 2_000_013]));
        update(&mut a, code(KeyCode::End), now());
        assert_eq!(selected_id(&a).as_deref(), Some("2000013"));
        update(&mut a, code(KeyCode::Enter), now());
        fetched(&mut a, dated([2_000_013, 2_000_012, 2_000_011, 1_995_008]));
        assert_eq!(row_ids(&a)[0], "2000013", "it moved to the top");
        assert_eq!(selected_id(&a).as_deref(), Some("2000013"));
        assert_eq!(a.mode, Mode::List);
        assert!(toast(&a).contains("reopens actions"));
        update(&mut a, code(KeyCode::Enter), now());
        let effects = update(&mut a, key('B'), now());
        assert!(
            matches!(&effects[..], [Effect::Open(url)] if url.ends_with("id=2000013")),
            "{effects:?}"
        );
    }

    #[test]
    fn filtering_and_view_changes_keep_the_selected_identity() {
        let mut a = app();
        let mut raw = a.snapshot.take().unwrap().raw;
        let mut other = app().snapshot.take().unwrap().raw.bugs.remove(0);
        other.id = BugId(2_000_099);
        other.summary = "Other work".into();
        raw.bugs.push(other);
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        update(&mut a, code(KeyCode::Down), now());
        let selected = a.snapshot.as_ref().unwrap().id(a.selected().unwrap()).to_string();
        update(&mut a, key('/'), now());
        for c in "nomatch".chars() {
            update(&mut a, key(c), now());
        }
        assert!(a.rows.is_empty());
        update(&mut a, code(KeyCode::Esc), now());
        assert_eq!(a.snapshot.as_ref().unwrap().id(a.selected().unwrap()).to_string(), selected);
        update(&mut a, code(KeyCode::Right), now());
        update(&mut a, code(KeyCode::Left), now());
        assert_eq!(a.snapshot.as_ref().unwrap().id(a.selected().unwrap()).to_string(), selected);
    }

    fn settings_page(a: &mut App) {
        update(a, key('s'), now());
        update(a, code(KeyCode::Tab), now());
        assert_eq!((a.mode, a.page), (Mode::Setup, Page::Settings));
    }

    fn go_to(a: &mut App, setting: Setting) {
        a.setting = settings::all().iter().position(|s| *s == setting).unwrap();
    }

    #[test]
    fn a_fetch_setting_saves_then_refreshes_and_the_page_stays_open() {
        let mut a = app();
        settings_page(&mut a);
        go_to(&mut a, Setting::LandingChecks);
        let effects = update(&mut a, code(KeyCode::Enter), now());
        assert!(!a.prefs.landing_checks);
        assert!(
            matches!(&effects[..], [Effect::SaveSetting { setting: Setting::LandingChecks, done }, Effect::Refresh]
                if done == "Landing checks off; refreshing"),
            "{effects:?}"
        );
        update(&mut a, Msg::Fetched(Ok(Box::new(fixture()))), now());
        assert_eq!((a.mode, a.page), (Mode::Setup, Page::Settings));
    }

    #[test]
    fn a_view_setting_saves_without_fetching_and_applies_at_once() {
        let mut a = app();
        settings_page(&mut a);
        go_to(&mut a, Setting::ShowHidden);
        let effects = update(&mut a, code(KeyCode::Char(' ')), now());
        assert!(matches!(&effects[..], [Effect::SaveSetting { .. }]), "{effects:?}");
        assert!(a.show_hidden);
    }

    #[test]
    fn hiding_the_current_section_returns_to_waiting() {
        let mut a = app();
        let sprint = Section::Sprint.index();
        a.switch_tab(sprint);
        settings_page(&mut a);
        go_to(&mut a, Setting::Section(sprint));
        update(&mut a, code(KeyCode::Enter), now());
        update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(a.prefs.sections[sprint], Show::Hidden);
        assert_eq!(a.section(), Section::Waiting);
        assert!(!a.tab_shown(sprint));
        assert_eq!(a.hidden_with_work(), vec![sprint]);
        update(&mut a, code(KeyCode::Esc), now());
        update(&mut a, key(char::from(b'1' + sprint as u8)), now());
        assert_eq!(a.section(), Section::Waiting, "its number key does nothing");
    }

    #[test]
    fn editing_takes_letters_and_esc_cancels_without_saving() {
        let mut a = app();
        settings_page(&mut a);
        go_to(&mut a, Setting::Whiteboard);
        update(&mut a, code(KeyCode::Enter), now());
        assert!(update(&mut a, key('q'), now()).is_empty(), "q is typed, not quit");
        assert_eq!(a.editing.as_deref(), Some("q"));
        assert!(update(&mut a, code(KeyCode::Esc), now()).is_empty());
        assert_eq!((a.editing.as_deref(), a.mode), (None, Mode::Setup));
        assert!(a.prefs.whiteboard.is_empty());
        update(&mut a, code(KeyCode::Enter), now());
        for c in "[ai]".chars() {
            update(&mut a, key(c), now());
        }
        let effects = update(&mut a, code(KeyCode::Enter), now());
        assert_eq!(a.cfg.whiteboard.as_deref(), Some("[ai]"));
        assert!(
            matches!(
                effects[0],
                Effect::SaveSetting {
                    setting: Setting::Whiteboard,
                    ..
                }
            ),
            "{effects:?}"
        );
    }

    #[test]
    fn auto_detail_tabs_hide_when_they_would_be_empty() {
        let mut a = app();
        assert!(a.selected().is_some_and(|e| e.drift.is_empty()));
        assert!(!a.pane_shown(Pane::Drift));
        a.prefs.panes[Pane::Drift.index()] = Show::Always;
        assert!(a.pane_shown(Pane::Drift));
        a.prefs.panes[Pane::Comments.index()] = Show::Hidden;
        assert!(!a.pane_shown(Pane::Comments));
        a.prefs.panes[Pane::Overview.index()] = Show::Hidden;
        assert!(a.pane_shown(Pane::Overview), "Overview can't be hidden");
    }
}
