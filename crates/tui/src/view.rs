use std::borrow::Cow;

use jiff::Timestamp;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use worklens_core::bugs::{State, days_between};
use worklens_core::drift::Level;
use worklens_core::marks::Hidden;
use worklens_core::present::{self, LinkKind, Segment, Signal, ago};
use worklens_core::raw::RevStatus;
use worklens_core::release;
use worklens_core::rules::{Reason, Tier};
use worklens_core::section::Section;
use worklens_core::snapshot::{ItemId, Scope, TryState};
use worklens_core::timeline;
use worklens_core::{Entry, Snapshot};

use crate::app::{App, Mode, Page, Pane, Tone};
use crate::keys;
use crate::settings::{self, Kind, Setting, ThemeName};

#[derive(Clone, Copy)]
struct Theme {
    quiet: Color,
    rule: Color,
    select: Option<Color>,
    accent: Color,
    ok: Color,
    warn: Color,
    error: Color,
}

impl Theme {
    fn of(tone: Tone) -> Self {
        let ansi = Self {
            quiet: Color::Reset,
            rule: Color::Reset,
            select: None,
            accent: Color::Cyan,
            ok: Color::Green,
            warn: Color::Yellow,
            error: Color::Red,
        };
        match tone {
            Tone::Dark => Self {
                quiet: Color::Indexed(246),
                rule: Color::Indexed(240),
                select: Some(Color::Indexed(237)),
                ..ansi
            },
            Tone::Light => Self {
                quiet: Color::Indexed(242),
                rule: Color::Indexed(250),
                select: Some(Color::Indexed(254)),
                ..ansi
            },
            Tone::Unknown => ansi,
            Tone::NoColor => Self {
                accent: Color::Reset,
                ok: Color::Reset,
                warn: Color::Reset,
                error: Color::Reset,
                ..ansi
            },
        }
    }

    fn quiet(self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), fg(self.quiet))
    }

    fn signal(self, signal: Signal) -> Style {
        match signal {
            Signal::Known => Style::new(),
            Signal::Unknown => fg(self.quiet),
            Signal::Good => fg(self.ok),
            Signal::Pending => fg(self.warn),
            Signal::Bad => fg(self.error),
        }
    }

    fn selection(self) -> Style {
        match self.select {
            Some(bg) => Style::new().bg(bg).add_modifier(Modifier::BOLD),
            None => fg(self.accent).add_modifier(Modifier::REVERSED | Modifier::BOLD),
        }
    }

    fn current_tab(self) -> Style {
        fg(self.accent).add_modifier(Modifier::BOLD)
    }

    fn cursor(self) -> &'static str {
        if self.select.is_some() { "▌" } else { ">" }
    }
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn fg(c: Color) -> Style {
    Style::new().fg(c)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

fn hint_line(t: Theme, pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(pairs.len() * 3);
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(t.quiet(keys::SEP));
        }
        spans.push(Span::raw((*key).to_owned()));
        if !label.is_empty() {
            spans.push(t.quiet(format!(" {label}")));
        }
    }
    Line::from(spans)
}

fn scroll_hint(t: Theme, first: usize, total: usize, shown: usize, pairs: &[(&str, &str)]) -> Line<'static> {
    let last = (first + shown).min(total);
    let mut line = hint_line(t, pairs);
    line.spans
        .insert(0, t.quiet(format!("{}–{last} of {total} lines{}", first + 1, keys::SEP)));
    line
}

pub fn fit(text: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let clipped = text.width() > n;
    let limit = if clipped { n - 1 } else { n };
    let mut out = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        if used + g.width() > limit {
            break;
        }
        out.push_str(g);
        used += g.width();
    }
    if clipped {
        out.push('…');
        used += 1;
    }
    out.push_str(&" ".repeat(n - used));
    out
}

fn input_tail(text: &str, cells: usize) -> String {
    if text.width() <= cells {
        return text.to_owned();
    }
    if cells == 0 {
        return String::new();
    }
    let mut start = text.len();
    let mut used = 1;
    for (i, g) in text.grapheme_indices(true).rev() {
        if used + g.width() > cells {
            break;
        }
        start = i;
        used += g.width();
    }
    format!("…{}", &text[start..])
}

fn short_email(email: &str) -> &str {
    email
        .strip_suffix("@mozilla.com")
        .or_else(|| email.strip_suffix("@mozilla.org"))
        .unwrap_or(email)
}

pub fn view(app: &App, f: &mut Frame<'_>) {
    if f.area().width < 60 || f.area().height < 16 {
        f.render_widget(
            Paragraph::new("worklens · Resize to 80×24 for the work queue. q quits.").wrap(Wrap { trim: false }),
            f.area(),
        );
        return;
    }
    let [header, tabs, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .areas(f.area());
    draw_header(app, f, header);
    if app.mode != Mode::Setup {
        draw_tabs(app, f, section_row(app, tabs, body));
    }
    match app.mode {
        Mode::Help => draw_help(app, f, body),
        Mode::Log => draw_log(app, f, body),
        Mode::Setup => draw_setup(app, f, body),
        _ => draw_body(app, f, body),
    }
    draw_status(app, f, status);
}

fn merge_when(days: i64) -> String {
    match days {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        n => format!("in {n} days"),
    }
}

fn draw_header(app: &App, f: &mut Frame<'_>, area: Rect) {
    let area = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    });
    let t = Theme::of(app.tone);
    let left = Line::from(Span::styled("worklens", bold()));
    let failures = || app.snapshot.as_ref().into_iter().flat_map(|s| s.raw.failures());
    let mut limited: Vec<&str> = failures().filter(|e| e.retry_at.is_some()).map(|e| e.source.as_str()).collect();
    limited.sort_unstable();
    limited.dedup();
    let mut down: Vec<&str> = failures()
        .map(|e| e.source.as_str())
        .filter(|source| !limited.contains(source))
        .collect();
    down.sort_unstable();
    down.dedup();
    let age = app
        .snapshot
        .as_ref()
        .map_or_else(String::new, |s| format!("Updated {}", ago(s.raw.fetched_at, app.now)));
    let busy = if app.busy.is_some() {
        format!("   {} Refreshing", SPINNER[app.frame % SPINNER.len()])
    } else {
        String::new()
    };
    let mut warnings = Vec::new();
    if app.error.is_some() {
        warnings.push("Refresh failed".to_owned());
    }
    if app.action_error.is_some() {
        warnings.push("Action failed".to_owned());
    }
    if !limited.is_empty() {
        warnings.push(format!("{} rate limited", limited.join(", ")));
    }
    if !down.is_empty() {
        warnings.push(format!("{} unavailable", down.join(", ")));
    }
    let mut right = Vec::new();
    if !warnings.is_empty() {
        let reserved = left.width() + age.width() + busy.width() + 16;
        let budget = (area.width as usize).saturating_sub(reserved);
        right.push(Span::styled(
            format!("{} · s Sources   ", fit(&format!("! {}", warnings.join("; ")), budget).trim_end()),
            fg(if app.error.is_some() || app.action_error.is_some() {
                t.error
            } else {
                t.warn
            }),
        ));
    }
    if let Some(s) = &app.snapshot {
        if warnings.is_empty()
            && app.busy.is_none()
            && let Some((_, days)) = s.merge().filter(|(_, d)| *d >= 0)
        {
            right.push(t.quiet(format!("Beta merge {}   ", merge_when(days))));
        }
        right.push(t.quiet(age));
    }
    if !busy.is_empty() {
        right.push(Span::styled(busy, fg(t.accent)));
    }
    let right = Line::from(right);
    let room = (area.width as usize).saturating_sub(left.width() + right.width() + 2 * HEADER_GAP);
    let mut left = left;
    if let Some(s) = &app.snapshot {
        let gauge = sprint_gauge(t, s, true, app.now);
        let line = if gauge.width() <= room {
            gauge
        } else {
            sprint_gauge(t, s, false, app.now)
        };
        if line.width() > 0 && line.width() <= room {
            left.spans.push(Span::raw(" ".repeat(HEADER_GAP)));
            left.spans.extend(line.spans);
        }
    }
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right), r);
}

const HEADER_GAP: usize = 3;

fn sprint_gauge(t: Theme, s: &Snapshot, full: bool, now: Timestamp) -> Line<'static> {
    let Some(sp) = &s.sprint else { return Line::default() };
    let over = sp.committed > sp.budget;
    let points = Span::styled(
        format!("{} of {} pts{}", sp.committed, sp.budget, if over { " · over budget" } else { "" }),
        if over { fg(t.warn) } else { Style::new() },
    );
    if !full {
        return Line::from(vec![t.quiet("Sprint "), points]);
    }
    const W: f32 = 10.0;
    let scale = sp.budget.max(sp.committed).max(1.0);
    let cells = |x: f32| (x / scale * W).round() as usize;
    let (done, committed, budget) = (cells(sp.done), cells(sp.committed), cells(sp.budget));
    let (part, part_color) = if t.select.is_some() {
        ("█", Color::Reset)
    } else {
        ("▓", t.quiet)
    };
    let full_block = "█";
    let empty = "░";
    let mut spans = vec![t.quiet("Sprint  ")];
    spans.push(Span::styled(full_block.repeat(done), fg(t.ok)));
    spans.push(Span::styled(
        part.repeat(committed.min(budget).saturating_sub(done)),
        fg(if over { t.warn } else { part_color }),
    ));
    spans.push(Span::styled(part.repeat(committed.saturating_sub(budget)), fg(t.warn)));
    spans.push(Span::styled(empty.repeat(budget.saturating_sub(committed)), fg(t.rule)));
    spans.push(Span::raw("  "));
    spans.push(points);
    let mut tail = String::new();
    if sp.done > 0.0 {
        tail.push_str(&format!(" · {} pts done", sp.done));
    }
    if let Some(end) = s.issue(sp.first).sprint.as_ref().and_then(|x| x.end_date) {
        let days = (-days_between(Some(end), now)).ceil();
        tail.push_str(&match days {
            d if d <= 0.0 => " · ends today".to_owned(),
            1.0 => " · 1 day left".to_owned(),
            d => format!(" · {d} days left"),
        });
    }
    spans.push(t.quiet(tail));
    if !sp.unestimated.is_empty() {
        spans.push(Span::styled(format!(" · {} unestimated", sp.unestimated.len()), fg(t.warn)));
    }
    Line::from(spans)
}

const TAB_GAP: usize = 3;

fn section_strip(app: &App, area: Rect) -> (Line<'static>, Line<'static>, Option<(u16, u16)>) {
    let area = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    });
    let t = Theme::of(app.tone);
    let right = if app.mode == Mode::Filter || !app.filter.is_empty() {
        Line::raw(format!(
            "/{}{}",
            input_tail(&app.filter, 22.min(area.width.saturating_sub(12) as usize)),
            if app.mode == Mode::Filter { "▏" } else { "" }
        ))
    } else if app.show_hidden {
        let mut line = hint_line(t, &[("Z", "hides")]);
        line.spans.insert(0, t.quiet(format!("Showing done and snoozed{}", keys::SEP)));
        line
    } else {
        let hidden: Vec<String> = app
            .hidden_with_work()
            .into_iter()
            .map(|i| format!("{} {}", Section::ALL[i].short(), app.counts[i]))
            .collect();
        if hidden.is_empty() {
            Line::default()
        } else {
            Line::from(t.quiet(format!("hidden: {}", hidden.join(", "))))
        }
    };
    let shown: Vec<usize> = (0..Section::COUNT).filter(|i| app.tab_shown(*i)).collect();
    let current = shown.iter().position(|i| *i == app.tab).unwrap_or(0);
    let label = |i: usize, short: bool| -> (&'static str, String) {
        let s = Section::ALL[i];
        let name = if short { s.short() } else { s.title() };
        let count = if app.snapshot.is_some() {
            format!(" {}", app.counts[i])
        } else {
            String::new()
        };
        (name, count)
    };
    let available = (area.width as usize).saturating_sub(right.width() + usize::from(right.width() > 0));
    let total = |labels: &[(&str, String)], gap: usize| -> usize {
        labels.iter().map(|(n, c)| n.width() + c.width()).sum::<usize>() + gap * labels.len().saturating_sub(1)
    };
    let long: Vec<(&str, String)> = shown.iter().map(|i| label(*i, false)).collect();
    let labels = if total(&long, TAB_GAP) <= available {
        long
    } else {
        shown.iter().map(|i| label(*i, true)).collect()
    };
    let gap = if total(&labels, TAB_GAP) <= available { TAB_GAP } else { 2 };
    let tab_width = |k: usize| labels[k].0.width() + labels[k].1.width();
    let mut start = current;
    let mut end = current + 1;
    let mut used = tab_width(current) + if total(&labels, gap) > available { 4 } else { 0 };
    loop {
        let mut added = false;
        if start > 0 && used + gap + tab_width(start - 1) <= available {
            start -= 1;
            used += gap + tab_width(start);
            added = true;
        }
        if end < labels.len() && used + gap + tab_width(end) <= available {
            used += gap + tab_width(end);
            end += 1;
            added = true;
        }
        if !added {
            break;
        }
    }
    let mut spans = Vec::new();
    let mut x = area.x as usize;
    let mut under = None;
    if start > 0 {
        spans.push(t.quiet("‹ "));
        x += 2;
    }
    for k in start..end {
        if k > start {
            spans.push(Span::raw(" ".repeat(gap)));
            x += gap;
        }
        let i = shown[k];
        let (name, count) = &labels[k];
        let has_work = app.snapshot.is_some() && app.counts[i] > 0;
        let drift = has_work && matches!(Section::ALL[i], Section::Drift | Section::Team);
        if k == current {
            spans.push(Span::styled(format!("{name}{count}"), t.current_tab()));
            under = Some((x as u16, (x + tab_width(k)) as u16));
        } else if drift {
            spans.push(Span::styled(*name, fg(t.warn)));
            spans.push(t.quiet(count.clone()));
        } else if has_work {
            spans.push(Span::raw(*name));
            spans.push(t.quiet(count.clone()));
        } else {
            spans.push(t.quiet(format!("{name}{count}")));
        }
        x += tab_width(k);
    }
    if end < labels.len() {
        spans.push(t.quiet(" ›"));
    }
    (Line::from(spans), right, under)
}

fn draw_tabs(app: &App, f: &mut Frame<'_>, area: Rect) {
    let (left, right, _) = section_strip(app, area);
    let area = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    });
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right), r);
}

fn underline_tab(t: Theme, f: &mut Frame<'_>, row: u16, span: (u16, u16), box_area: Rect) {
    let from = span.0.max(box_area.x + 1);
    let to = span.1.min(box_area.right().saturating_sub(1));
    let buf = f.buffer_mut();
    for x in from..to {
        buf[(x, row)].set_symbol("━").set_style(fg(t.accent));
    }
}

fn drift_trend(app: &App, total: usize) -> String {
    let today = app.now.to_zoned(jiff::tz::TimeZone::system()).date();
    let Some(then) = app.drift_log.baseline(today) else {
        return String::new();
    };
    let was = match app.section() {
        Section::Drift => then.mine,
        Section::Team => then.team,
        _ => return String::new(),
    };
    let on = then.date.strftime("%b %d");
    match total.cmp(&was) {
        std::cmp::Ordering::Less => format!("down from {was} on {on}"),
        std::cmp::Ordering::Greater => format!("up from {was} on {on}"),
        std::cmp::Ordering::Equal => format!("same as {on}"),
    }
}

fn block(color: Color) -> Block<'static> {
    Block::bordered().border_type(BorderType::Rounded).border_style(fg(color))
}

fn body_panes(app: &App, area: Rect) -> [Rect; 2] {
    if app.wide() {
        Layout::horizontal([Constraint::Percentage(55), Constraint::Fill(1)]).areas(area)
    } else {
        let [list, _, side] = Layout::vertical([
            Constraint::Length(app.list_rows() as u16 + 2),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(area);
        [list, side]
    }
}

fn section_row(app: &App, row: Rect, body: Rect) -> Rect {
    if app.wide() && app.mode != Mode::Setup {
        Rect::new(row.x, row.y, body_panes(app, body)[0].width, 1)
    } else {
        row
    }
}

pub fn detail_paging(app: &App) -> (u16, u16) {
    let area = Rect::new(0, 2, app.size.0, app.size.1.saturating_sub(3));
    let side = body_panes(app, area)[1];
    let width = side.width.saturating_sub(4);
    let height = side.height.saturating_sub(3).max(1);
    let total = detail_content(app, width).line_count(width);
    (
        height.saturating_sub(1).max(1),
        total.saturating_sub(height as usize).min(u16::MAX as usize) as u16,
    )
}

fn detail_content(app: &App, width: u16) -> Paragraph<'_> {
    let width = width as usize;
    if app.current_pane() == Pane::Telemetry {
        let mut lines = vec![
            Line::raw("Firefox Desktop metric definitions that name this bug"),
            Line::raw("Definitions, not collected values. All of yours: worklens telemetry"),
        ];
        if let Some((s, report)) = app.snapshot.as_ref().and_then(|s| s.raw.telemetry.as_ref().map(|r| (s, r))) {
            lines.push(Line::raw(format!("Fetched {}", report.fetched_at)));
            for error in s.raw.errors.iter().filter(|e| e.source == "probeinfo") {
                lines.push(Line::raw(format!("Cached definitions; latest fetch failed: {}", error.message)));
            }
            for metric in app.item_metrics() {
                let evidence = worklens_core::evidence::metric(s, metric, report.fetched_at);
                lines.push(Line::raw(""));
                lines.push(Line::styled(evidence.label, bold()));
                lines.push(Line::raw(evidence.summary));
                lines.push(Line::raw(evidence.url));
                for url in &metric.data_reviews {
                    lines.push(Line::raw(format!("Data-review reference (not approval proof): {url}")));
                }
            }
        }
        return Paragraph::new(lines).wrap(Wrap { trim: false });
    }
    let lines = match (&app.snapshot, app.selected()) {
        (Some(s), Some(e)) => match app.current_pane() {
            Pane::Overview => overview(app, s, e, width),
            Pane::Timeline => timeline_lines(app, s, e, width),
            Pane::Comments => comments(app, s, e, width),
            Pane::Links => links(Theme::of(app.tone), s, e),
            Pane::Drift => drift_lines(Theme::of(app.tone), e),
            Pane::Telemetry => unreachable!("drawn above"),
        },
        (Some(_), None) if !app.rows.is_empty() => vec![Line::raw("No item selected.")],
        _ => Vec::new(),
    };
    Paragraph::new(lines).wrap(Wrap { trim: false })
}

fn draw_body(app: &App, f: &mut Frame<'_>, area: Rect) {
    let [list, side] = body_panes(app, area);
    draw_list(app, f, list);
    let strip = section_row(app, Rect::new(area.x, area.y.saturating_sub(1), area.width, 1), area);
    if let (_, _, Some(span)) = section_strip(app, strip) {
        underline_tab(Theme::of(app.tone), f, list.y, span, list);
    }
    let actions = if app.mode == Mode::Menu { app.actions() } else { Vec::new() };
    if app.mode == Mode::Menu && menu_needs_body(app, &actions, side) {
        draw_menu(app, f, area, &actions);
    } else if app.mode == Mode::Menu {
        draw_menu(app, f, side, &actions);
    } else {
        draw_detail(app, f, side);
    }
}

fn draw_list(app: &App, f: &mut Frame<'_>, area: Rect) {
    let t = Theme::of(app.tone);
    let total = app.snapshot.as_ref().map_or(0, |s| s.section(app.section(), true).count());
    let available = app
        .snapshot
        .as_ref()
        .map_or(0, |s| s.section(app.section(), app.show_hidden).count());
    let hidden = total.saturating_sub(available);
    let mut label = Line::default();
    if app.snapshot.is_some() && !app.filter.is_empty() {
        label = hint_line(t, &[("Esc", "clears ")]);
        let matches = format!(" {} of {} match{}", app.rows.len(), available, keys::SEP);
        label.spans.insert(0, t.quiet(matches));
    } else if app.snapshot.is_some() {
        let trend = drift_trend(app, total);
        if hidden > 0 {
            label = hint_line(t, &[("Z", "shows ")]);
            label.spans.insert(0, t.quiet(format!("{hidden} hidden{}", keys::SEP)));
        }
        match (trend.is_empty(), hidden > 0) {
            (true, true) => label.spans.insert(0, Span::raw(" ")),
            (false, true) => label.spans.insert(0, t.quiet(format!(" {trend}{}", keys::SEP))),
            (false, false) => label = Line::from(t.quiet(format!(" {trend} "))),
            (true, false) => {}
        }
    }
    let outer = block(t.rule).title_bottom(label);
    let inner = outer.inner(area);
    f.render_widget(outer, area);
    let Some(s) = &app.snapshot else {
        let mut lines = Vec::new();
        if let Some(e) = &app.error {
            lines.push(Line::raw("Couldn't fetch your work."));
            lines.push(Line::styled(e.clone(), fg(t.error)));
        } else if let Some(step) = &app.busy {
            lines.push(Line::raw("Fetching your work…"));
            if step != "Refreshing" {
                lines.push(Line::from(t.quiet(step.clone())));
            }
        } else {
            lines.push(Line::raw("Nothing fetched yet. r fetches now."));
        }
        lines.push(Line::from(t.quiet("No cached work yet. s opens Sources; r retries after failure.")));
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::new().padding(Padding::horizontal(1))),
            inner,
        );
        return;
    };
    let section = app.section();
    if app.rows.is_empty() {
        let text = if !app.filter.is_empty() {
            format!("No matches for “{}”. Esc clears the filter.", app.filter)
        } else if hidden > 0 {
            format!("{hidden} items hidden locally (done or snoozed). Z shows them.")
        } else if let Some(why) = s.not_checked(section) {
            format!("{why} s opens Sources.")
        } else if let Some(error) = s.raw.failures().next() {
            format!("Work may be missing: {} unavailable. s opens Sources.", error.source)
        } else if app.error.is_some() {
            "Refresh failed; cached queue may be incomplete. s opens Sources.".to_owned()
        } else {
            section.empty().to_owned()
        };
        let mut lines = vec![Line::raw(text)];
        let next = (app.tab + 1..Section::COUNT)
            .find(|i| app.counts[*i] > 0)
            .map(|i| Section::ALL[i])
            .filter(|s| !matches!(s, Section::Other | Section::Team));
        if let Some(next) = next {
            lines.push(Line::from(t.quiet(format!(
                "Next: {} ({}) →",
                next.title(),
                app.counts[next.index()]
            ))));
        }
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::new().padding(Padding::horizontal(1))),
            inner,
        );
        return;
    }
    let height = inner.height as usize;
    let cursor = app.cursor();
    let stack_of = |n: usize| s.entry(app.rows[n]).stack;
    let mut shown: Vec<Shown> = Vec::with_capacity(app.rows.len());
    let mut at = 0;
    for n in 0..app.rows.len() {
        if let Some(k) = stack_of(n).filter(|k| n == 0 || stack_of(n - 1) != Some(*k)) {
            shown.push(Shown::Head(k));
        }
        if n == cursor {
            at = shown.len();
        }
        let branch = stack_of(n).map(|k| n + 1 == app.rows.len() || stack_of(n + 1) != Some(k));
        shown.push(Shown::Row(n, branch));
    }
    let top = at.saturating_sub(height / 2).min(shown.len().saturating_sub(height));
    let visible = &shown[top..shown.len().min(top + height)];
    if shown.len() > height {
        let numbers = || {
            visible
                .iter()
                .filter_map(|x| if let Shown::Row(n, _) = x { Some(*n + 1) } else { None })
        };
        let first = numbers().next().unwrap_or(0);
        let last = numbers().next_back().unwrap_or(0);
        let text = format!(" {first}–{last} of {} ", app.rows.len());
        let w = (text.width() as u16).min(area.width.saturating_sub(2));
        f.render_widget(
            Paragraph::new(t.quiet(text)),
            Rect::new(area.right().saturating_sub(w + 1), area.bottom().saturating_sub(1), w, 1),
        );
    }
    let id_width = app
        .rows
        .iter()
        .map(|i| s.id(s.entry(*i)).to_string().width())
        .max()
        .unwrap_or(7)
        .clamp(7, 16);
    let width = inner.width as usize;
    let lines: Vec<Line<'_>> = visible
        .iter()
        .map(|x| match *x {
            Shown::Head(k) => {
                let text = worklens_core::stack::header(s, k);
                let lead = " ".repeat(STACK_INDENT);
                Line::from(vec![
                    Span::raw(lead),
                    Span::styled("┌ ", fg(t.rule)),
                    t.quiet(fit(&text, width.saturating_sub(STACK_INDENT + 2)).trim_end().to_owned()),
                ])
            }
            Shown::Row(n, branch) => row(app, t, s, s.entry(app.rows[n]), n == cursor, (width, id_width), branch),
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

const STACK_INDENT: usize = 16;

#[derive(Clone, Copy)]
enum Shown {
    Head(worklens_core::StackIdx),
    Row(usize, Option<bool>),
}

fn try_mark(t: Theme, s: &Snapshot, e: &Entry) -> Span<'static> {
    let state = present::try_revision(s, e).map(|r| s.try_state(r));
    match state {
        Some(TryState::Push { push, .. }) if push.failed > 0 => Span::styled("✗", fg(t.error)),
        Some(TryState::Push { push, .. }) if push.running > 0 => Span::styled("○", fg(t.warn)),
        Some(TryState::Push { outdated: true, .. }) => Span::styled("~", fg(t.warn)),
        Some(TryState::Push { .. }) => Span::styled("✓", fg(t.ok)),
        _ => Span::raw(" "),
    }
}

fn row<'a>(
    app: &App,
    t: Theme,
    s: &'a Snapshot,
    e: &'a Entry,
    selected: bool,
    (width, id_width): (usize, usize),
    branch: Option<bool>,
) -> Line<'a> {
    let p = e.placement;
    let lead = if selected {
        Span::styled(t.cursor(), fg(t.accent).add_modifier(Modifier::BOLD))
    } else if e.marks.is_new {
        Span::styled("•", fg(t.accent))
    } else {
        Span::raw(" ")
    };
    let days = days_between(present::age_since(s, e), app.now).floor();
    let age = if days.is_finite() {
        format!("{}d", days.max(0.0))
    } else {
        String::new()
    };
    let tag: Cow<'_, str> = match (e.marks.hidden, p.map(|p| p.reason)) {
        (Some(Hidden::Done), _) => Cow::Borrowed("done"),
        (Some(Hidden::Snoozed), _) => Cow::Borrowed("zz"),
        (None, _) if app.section() == Section::Sprint => match e.issue.and_then(|i| s.issue(i).points) {
            Some(pts) => Cow::Owned(format!("{pts}p")),
            None => Cow::Borrowed("?p"),
        },
        (None, Some(Reason::Sprint(_))) => Cow::Owned(s.tag(e).to_string()),
        (None, reason) => Cow::Borrowed(reason.map_or("", Reason::tag)),
    };
    let tier = p.filter(|p| !matches!(p.reason, Reason::Landed(_))).map(|p| p.tier);
    let tag_color = match tier {
        Some(Tier::Critical) => t.error,
        Some(Tier::Important) => t.warn,
        _ => t.quiet,
    };
    let drift = if e.drift.is_empty() {
        Span::raw(" ")
    } else {
        Span::styled("⚑", fg(if e.has_error_drift() { t.error } else { t.warn }))
    };
    let used = STACK_INDENT + id_width + 1 + if branch.is_some() { 2 } else { 0 };
    let dim_title = !selected && (app.section() == Section::Other || e.marks.hidden.is_some());
    let mut title = Style::new();
    if selected {
        title = title.add_modifier(Modifier::BOLD);
    }
    if dim_title {
        title = title.fg(t.quiet);
    }
    let mut line = Line::from(vec![
        lead,
        Span::styled(
            format!("{} ", fit(&age, 4)),
            fg(if p.is_some_and(|p| p.stale) { t.warn } else { t.quiet }),
        ),
        Span::styled(
            format!(
                "{}{} ",
                match tier {
                    Some(Tier::Critical) => "!",
                    Some(Tier::Important) => "*",
                    _ => " ",
                },
                fit(&tag, 4)
            ),
            fg(tag_color),
        ),
        try_mark(t, s, e),
        Span::raw(" "),
        drift,
        Span::raw(" "),
        Span::styled(
            match branch {
                Some(true) => "└ ",
                Some(false) => "├ ",
                None => "",
            },
            fg(t.rule),
        ),
        Span::styled(fit(&s.id(e).to_string(), id_width), fg(t.quiet)),
        Span::raw(" "),
        Span::styled(fit(s.title(e), width.saturating_sub(used)), title),
    ]);
    if !selected {
        return line;
    }
    if t.select.is_some() {
        for span in &mut line.spans {
            if span.style.fg == Some(t.quiet) {
                span.style = span.style.fg(Color::Reset);
            }
        }
    } else {
        for span in &mut line.spans {
            span.style = span.style.fg(t.accent);
        }
    }
    line.style(t.selection())
}

fn draw_detail_tabs(app: &App, t: Theme, f: &mut Frame<'_>, box_area: Rect) {
    let shown: Vec<Pane> = Pane::ALL.into_iter().filter(|p| app.pane_shown(*p)).collect();
    let titles: Vec<&str> = shown.iter().map(|p| p.title()).collect();
    let current = shown.iter().position(|p| *p == app.current_pane()).unwrap_or(0);
    draw_box_tabs(t, f, &titles, current, box_area);
}

fn draw_box_tabs(t: Theme, f: &mut Frame<'_>, titles: &[&'static str], current: usize, box_area: Rect) {
    let row = Rect::new(box_area.x + 1, box_area.y.saturating_sub(1), box_area.width.saturating_sub(2), 1);
    let width = row.width as usize;
    let mut names = Vec::with_capacity(titles.len() * 2 + 2);
    let mut used = 0;
    let mut under = (0, 0);
    for (i, title) in titles.iter().enumerate() {
        if i > 0 {
            names.push(Span::raw(" ".repeat(TAB_GAP)));
            used += TAB_GAP;
        }
        let w = title.width();
        if i == current {
            names.push(Span::styled(*title, t.current_tab()));
            under = (used, used + w);
        } else {
            names.push(t.quiet(*title));
        }
        used += w;
    }
    const HINT: &str = "Tab ⇥";
    if used + 2 + HINT.width() <= width {
        names.push(Span::raw(" ".repeat(width - used - HINT.width())));
        names.push(t.quiet(HINT));
    }
    f.render_widget(Clear, row);
    f.render_widget(Paragraph::new(Line::from(names)), row);
    let span = (row.x + under.0 as u16, row.x + under.1 as u16);
    underline_tab(t, f, box_area.y, span, box_area);
}

fn draw_detail(app: &App, f: &mut Frame<'_>, area: Rect) {
    let t = Theme::of(app.tone);
    let outer = block(t.rule).padding(Padding::horizontal(1));
    let inner = outer.inner(area);
    f.render_widget(outer, area);
    if app.mode == Mode::Snooze {
        let mut lines = vec![
            Line::styled("Snooze locally", bold()),
            Line::raw("Until a date: tomorrow, mon, 3d, nov 10"),
            Line::raw("Enter alone: until new activity. Esc cancels."),
            Line::raw(""),
            Line::styled(
                format!("> {}▏", input_tail(&app.snooze_text, inner.width.saturating_sub(3) as usize)),
                fg(t.accent),
            ),
        ];
        if app.toast.as_ref().is_some_and(|t| t.error) {
            lines.push(Line::styled("Date not understood. Try a date above or Esc to cancel.", fg(t.error)));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }
    if app.selected().is_some() {
        draw_detail_tabs(app, t, f, area);
    }
    let [content, overflow] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
    let paragraph = detail_content(app, content.width);
    let total = paragraph.line_count(content.width);
    let max = total.saturating_sub(content.height as usize);
    let scroll = (app.pane_scroll as usize).min(max) as u16;
    f.render_widget(paragraph.scroll((scroll, 0)), content);
    if max > 0 {
        let hint = scroll_hint(t, scroll as usize, total, content.height as usize, keys::DETAIL_SCROLL_HINTS);
        f.render_widget(Paragraph::new(hint), overflow);
    }
}

fn lands_in(l: worklens_core::train::LandsIn, now: Timestamp) -> Option<(String, i64)> {
    let (by, after) = (l.if_by?, l.after?);
    let days = i64::from(now.to_zoned(jiff::tz::TimeZone::UTC).date().until(by).ok()?.get_days());
    let when = match days {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        n => format!("in {n} days"),
    };
    let by = by.strftime("%b %-d");
    Some((
        format!("Expected Firefox {} if landed by {by} ({when}); after that, {after}.", l.version),
        days,
    ))
}

fn is_set(v: &str) -> bool {
    !v.is_empty() && v != "--" && v != "N/A"
}

const LABEL: usize = 13;

fn wrap(text: &str, cells: usize) -> Vec<String> {
    let cells = cells.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let gap = usize::from(!line.is_empty());
        if !line.is_empty() && line.width() + gap + word.width() > cells {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        for g in word.graphemes(true) {
            if line.width() + g.width() > cells {
                lines.push(std::mem::take(&mut line));
            }
            line.push_str(g);
        }
    }
    lines.push(line);
    lines
}

fn table_row(label: &str, label_style: Style, label_width: usize, value: &str, value_style: Style, width: usize) -> Vec<Line<'static>> {
    wrap(value, width.saturating_sub(label_width).max(8))
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let head = if i == 0 {
                format!("{} ", fit(label, label_width.saturating_sub(1)))
            } else {
                " ".repeat(label_width)
            };
            Line::from(vec![Span::styled(head, label_style), Span::styled(part, value_style)])
        })
        .collect()
}

fn strip_row(t: Theme, label: &str, segments: &[(String, Style)], label_width: usize, width: usize) -> Vec<Line<'static>> {
    let room = width.saturating_sub(label_width).max(8);
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used = 0;
    for (text, style) in segments {
        if used > 0 && used + 3 + text.width() > room {
            lines.push(Vec::new());
            used = 0;
        } else if used > 0 {
            lines
                .last_mut()
                .expect("starts with one line")
                .push(Span::styled(" · ", fg(t.rule)));
            used += 3;
        }
        for (i, part) in wrap(text, room).into_iter().enumerate() {
            if i > 0 {
                lines.push(Vec::new());
                used = 0;
            }
            used += part.width();
            lines.last_mut().expect("starts with one line").push(Span::styled(part, *style));
        }
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(i, mut spans)| {
            let head = if i == 0 {
                format!("{} ", fit(label, label_width.saturating_sub(1)))
            } else {
                " ".repeat(label_width)
            };
            spans.insert(0, t.quiet(head));
            Line::from(spans)
        })
        .collect()
}

fn styled(t: Theme, segments: Vec<Segment>) -> Vec<(String, Style)> {
    segments.into_iter().map(|s| (s.text, t.signal(s.signal))).collect()
}

fn release_matrix(t: Theme, channels: &[release::Channel], width: usize) -> Vec<Line<'static>> {
    use GridCell::{Label, Value};
    const FIELDS: [&str; 3] = ["Reported", "Uplift", "Landing"];
    let cells = |c: &release::Channel| [c.reported, c.uplift, c.landing];
    let mut columns = vec![
        std::iter::once(Label(String::new()))
            .chain(channels.iter().map(|c| Label(c.name.clone())))
            .collect::<Vec<_>>(),
    ];
    for (n, field) in FIELDS.iter().enumerate() {
        columns.push(
            std::iter::once(Label((*field).to_owned()))
                .chain(channels.iter().map(|c| Value(cells(c)[n])))
                .collect(),
        );
    }
    let mut rows = vec![
        std::iter::once(Label(String::new()))
            .chain(FIELDS.iter().map(|f| Label((*f).to_owned())))
            .collect::<Vec<_>>(),
    ];
    for c in channels {
        rows.push(
            std::iter::once(Label(c.name.clone()))
                .chain(cells(c).into_iter().map(Value))
                .collect(),
        );
    }
    for table in [&columns, &rows] {
        for gap in [2, 1] {
            if let Some(lines) = grid(t, table, gap, width) {
                return lines;
            }
        }
    }
    let name_width = channels.iter().map(|c| c.name.width() + 2).max().unwrap_or(0).min(width / 2);
    channels
        .iter()
        .flat_map(|c| {
            let segments: Vec<(String, Style)> = FIELDS
                .iter()
                .zip(cells(c))
                .map(|(f, x)| (format!("{} {}", f.to_lowercase(), x.text), t.signal(x.signal)))
                .collect();
            strip_row(t, &c.name, &segments, name_width, width)
        })
        .collect()
}

enum GridCell {
    Label(String),
    Value(release::Cell),
}

impl GridCell {
    fn text(&self) -> &str {
        match self {
            Self::Label(text) => text,
            Self::Value(cell) => cell.text,
        }
    }
}

fn grid(t: Theme, table: &[Vec<GridCell>], gap: usize, width: usize) -> Option<Vec<Line<'static>>> {
    let count = table.first()?.len();
    let mut widths: Vec<usize> = (0..count)
        .map(|n| table.iter().map(|row| row[n].text().width()).max().unwrap_or(0) + gap)
        .collect();
    widths[0] = widths[0].max(LABEL);
    widths[count - 1] -= gap;
    if widths.iter().sum::<usize>() > width {
        return None;
    }
    let line = |row: &Vec<GridCell>| {
        let spans: Vec<Span<'static>> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| match cell {
                GridCell::Label(text) => t.quiet(fit(text, *w)),
                GridCell::Value(c) => Span::styled(fit(c.text, *w), t.signal(c.signal)),
            })
            .collect();
        Line::from(spans)
    };
    Some(table.iter().map(line).collect())
}

fn evidence_rows(workflow: &worklens_core::evidence::Workflow, every: bool, matrix: bool, now: Timestamp) -> Vec<(String, String)> {
    use worklens_core::evidence::Evidence;
    let summary = |evidence: &Evidence| {
        if matches!(evidence.source, "treeherder-landings" | "probeinfo") {
            format!("{} · checked {}", evidence.summary, ago(evidence.observed_at, now))
        } else {
            evidence.summary.clone()
        }
    };
    let keep = |e: &&Evidence| (every || e.is_known()) && !e.is_reviewer();
    let mut rows: Vec<(String, String)> = workflow
        .approvals
        .iter()
        .filter(keep)
        .filter(|e| !(matrix && e.is_uplift()))
        .map(|e| (e.label.clone(), summary(e)))
        .collect();
    let channels: Vec<&Evidence> = workflow.channels.iter().filter(keep).filter(|_| !matrix).collect();
    if channels.len() > 1 && channels.iter().all(|c| c.summary == channels[0].summary) {
        let names: Vec<&str> = channels.iter().map(|c| c.label.as_str()).collect();
        rows.push(("Channels".into(), format!("{}: {}", names.join(", "), channels[0].summary)));
    } else {
        rows.extend(channels.iter().map(|e| (e.label.clone(), summary(e))));
    }
    rows.extend(
        workflow
            .landings
            .iter()
            .chain(&workflow.telemetry)
            .filter(keep)
            .map(|e| (e.label.clone(), summary(e))),
    );
    if every {
        for (n, limitation) in workflow.limitations.iter().enumerate() {
            let label = if n == 0 { "Unknown" } else { "" };
            rows.push((label.into(), limitation.clone()));
        }
    } else if !rows.is_empty() && !workflow.unknown.is_empty() {
        rows.push(("Unknown".into(), workflow.unknown.join(", ")));
    }
    rows
}

fn divider(t: Theme, label: &str, width: usize) -> Line<'static> {
    let label = fit(label, width.saturating_sub(5)).trim_end().to_owned();
    let rest = width.saturating_sub(label.width() + 4);
    Line::from(vec![
        Span::styled("── ", fg(t.rule)),
        t.quiet(label),
        Span::styled(format!(" {}", "─".repeat(rest)), fg(t.rule)),
    ])
}

fn overview<'a>(app: &App, s: &'a Snapshot, e: &'a Entry, width: usize) -> Vec<Line<'a>> {
    let t = Theme::of(app.tone);
    let drift_first = matches!(app.section(), Section::Drift | Section::Team);
    let mut out: Vec<Line<'a>> = vec![Line::styled(s.title(e), bold())];
    let push_drift = |out: &mut Vec<Line<'a>>| {
        for d in &e.drift {
            let (mark, color) = if d.level == Level::Error { ("✗", t.error) } else { ("~", t.warn) };
            out.push(Line::styled(format!("{mark} {}", d.message), fg(color)));
            out.push(Line::from(vec![t.quiet("Why: "), Span::raw(d.cause.clone())]));
        }
    };
    if drift_first {
        push_drift(&mut out);
    }
    if let (Some(p), false) = (e.placement, drift_first) {
        let where_ = if app.section() == Section::Sprint && p.section != Section::Sprint {
            format!("{}: ", p.section.title())
        } else {
            String::new()
        };
        out.push(Line::styled(
            format!("{where_}{}{}", if p.stale { "Waited: " } else { "" }, s.reason(e)),
            if p.stale { fg(t.warn) } else { Style::new() },
        ));
        if let Some((text, days)) = s.lands_in(e).and_then(|l| lands_in(l, s.now)) {
            out.push(Line::styled(text, fg(if days <= 2 { t.warn } else { t.quiet })));
        } else if matches!(p.reason, Reason::Accepted(_)) {
            out.push(Line::from("Release estimate unavailable: calendar missing or stale"));
        }
    }
    if let Some(step) = s.next_step(e) {
        out.push(Line::styled(format!("Next: {step}"), fg(t.accent).add_modifier(Modifier::BOLD)));
    }
    if let Some(place) = worklens_core::stack::position(s, e) {
        out.push(Line::raw(place));
    }
    for d in worklens_core::dependencies::shown(s, e) {
        let owner = d.owner.as_deref().unwrap_or("owner unknown");
        out.push(Line::raw(format!("Depends: {} · {} · {owner}", d.id, d.status)));
        if let Some(title) = d.title {
            out.push(Line::from(t.quiet(title)));
        }
    }
    if !e.changes.is_empty() {
        out.push(Line::from(t.quiet("Since you looked:")));
        for change in &e.changes {
            out.push(Line::from(vec![Span::styled("• ", fg(t.accent)), Span::raw(change.clone())]));
        }
    }
    match e.marks.hidden {
        Some(Hidden::Done) => out.push(Line::from(t.quiet("Done; back when it has new activity. d to bring it back now."))),
        Some(Hidden::Snoozed) => {
            let until = e.marks.snoozed_until.map_or_else(
                || "until new activity".to_owned(),
                |t| format!("until {}", present::date(t, &jiff::tz::TimeZone::system())),
            );
            out.push(Line::from(t.quiet(format!("Snoozed {until}. z to bring it back now."))));
        }
        None => {}
    }

    out.push(Line::raw(""));
    let row = |out: &mut Vec<Line<'a>>, label: &str, value: &str, style: Style| {
        out.extend(table_row(label, fg(t.quiet), LABEL, value, style, width));
    };
    let tracked = s.tracked_for(e);
    if !tracked.is_empty() {
        let text = format!("{}: release management is watching it", tracked.join(", "));
        row(
            &mut out,
            "Tracked",
            &text,
            if e.placement.is_some_and(|p| matches!(p.reason, Reason::Landed(_))) {
                Style::new()
            } else {
                fg(t.warn)
            },
        );
    }
    if let Some(b) = e.bug.map(|b| s.bug(b)) {
        let mut parts = vec![b.id.to_string(), State(b).to_string()];
        parts.extend([&b.severity, &b.priority].into_iter().filter(|v| is_set(v)).cloned());
        parts.push(format!("{} :: {}", b.product, b.component));
        row(&mut out, "Bugzilla", &parts.join(" · "), Style::new());
        if let Some(text) = timeline::ships(s, e) {
            row(&mut out, "Release", &text, Style::new());
        }
    }
    if let Some(i) = e.issue.map(|i| s.issue(i)) {
        let mut text = format!("{} · {}", i.key, i.status);
        if let Some(p) = i.points {
            text.push_str(&format!(" · {p} {}", if (p - 1.0).abs() < f32::EPSILON { "pt" } else { "pts" }));
        }
        if let Some(sp) = &i.sprint {
            text.push_str(&format!(" · {}", sp.name));
        }
        row(&mut out, "Jira", &text, Style::new());
    }
    if let Some(p) = s.parent(e) {
        let label = p.kind.as_deref().unwrap_or("Parent");
        row(&mut out, label, &format!("{} {} · {}", p.key, p.summary, p.status), Style::new());
    }
    if let ItemId::Idea(i) = e.id {
        idea_rows(&mut out, t, s, i, e.scope, width);
    } else if let Some(idea) = e.idea.map(|i| s.idea(i)) {
        let mut parts = vec![format!("{} {}", idea.key, idea.summary), idea.status.clone()];
        parts.extend(idea.target.clone());
        row(&mut out, "Idea", &parts.join(" · "), Style::new());
    }
    for r in &e.revisions {
        let rev = s.rev(*r);
        let mut parts = vec![rev.id.to_string(), rev.status.as_str().to_owned()];
        if !s.is_mine(rev) {
            parts.push(format!(
                "by {}",
                s.raw.users.get(&rev.author).map_or("someone else", String::as_str)
            ));
        }
        row(&mut out, "Phabricator", &parts.join(" · "), Style::new());
        if matches!(rev.status, RevStatus::Published | RevStatus::Abandoned) {
            continue;
        }
        let reviews = styled(t, present::review_segments(s, rev));
        out.extend(strip_row(t, "  Reviewers", &reviews, LABEL, width));
        let state = s.try_state(*r);
        let mut tries = styled(t, present::try_segments(state));
        if let TryState::Push { push, .. } = state {
            tries.push((format!("pushed {}", ago(push.pushed_at, app.now)), fg(t.quiet)));
        }
        if !tries.is_empty() {
            out.extend(strip_row(t, "  Try", &tries, LABEL, width));
        }
    }
    if let Some(r) = e.review.map(|r| s.rev(r)) {
        let by = s.raw.users.get(&r.author).map(|a| format!(" by {a}")).unwrap_or_default();
        row(&mut out, "Review", &format!("{} {}{by}", r.id, r.title), Style::new());
        let reviews = styled(t, present::review_segments(s, r));
        out.extend(strip_row(t, "  Reviewers", &reviews, LABEL, width));
    }
    if e.scope == Scope::Team {
        row(&mut out, "Owner", present::owner(s, e).unwrap_or("nobody"), Style::new());
    }
    let matrix = release::shown(s, e).then(|| release::matrix(s, e)).flatten();
    if let Some(channels) = &matrix {
        out.push(Line::raw(""));
        out.push(divider(t, "Release channels", width));
        out.extend(release_matrix(t, channels, width));
    }
    let workflow = worklens_core::evidence::workflow(s, e);
    let rows = evidence_rows(&workflow, worklens_core::evidence::decides(e), matrix.is_some(), app.now);
    if !rows.is_empty() {
        out.push(Line::raw(""));
        out.push(divider(t, "Approvals and fix presence", width));
        let longest = rows.iter().map(|(label, _)| label.width()).max().unwrap_or(0);
        let longest_word = rows
            .iter()
            .flat_map(|(_, value)| value.split(' '))
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0);
        let cap = (width * 2 / 5).min(width.saturating_sub(longest_word)).max(LABEL);
        let label_width = (longest + 1).clamp(LABEL, cap);
        for (label, value) in &rows {
            out.extend(table_row(label, fg(t.quiet), label_width, value, Style::new(), width));
        }
    }
    if !drift_first && !e.drift.is_empty() {
        out.push(Line::raw(""));
        let title = if matches!(e.id, ItemId::Idea(_)) {
            "The idea and your work disagree"
        } else {
            "Bugzilla and Jira disagree"
        };
        out.push(divider(t, title, width));
        push_drift(&mut out);
    }

    let asked = s.needinfo_comment(e);
    if let Some(c) = asked {
        out.push(Line::raw(""));
        out.push(divider(
            t,
            &format!("{} asked, {}", short_email(&c.author), ago(c.time, app.now)),
            width,
        ));
        out.extend(
            c.text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(3)
                .map(|l| Line::raw(l.to_owned())),
        );
    }
    let shown = |c: &&worklens_core::raw::Comment| asked.is_none_or(|a| a.author != c.author || a.time != c.time);
    if let Some(c) = s.last_comment(e).filter(shown) {
        out.push(Line::raw(""));
        out.push(divider(t, &format!("{}, {}", short_email(&c.author), ago(c.time, app.now)), width));
        let text: Vec<&str> = c.text.lines().filter(|l| !l.trim().is_empty()).take(8).collect();
        for l in text {
            out.push(Line::raw(l.to_owned()));
        }
    }
    out
}

fn timeline_lines<'a>(app: &App, s: &'a Snapshot, e: &'a Entry, width: usize) -> Vec<Line<'a>> {
    let t = Theme::of(app.tone);
    let tz = jiff::tz::TimeZone::system();
    let year = app.now.to_zoned(tz.clone()).year();
    let events = timeline::events(s, e);
    let days: Vec<String> = events
        .iter()
        .map(|ev| {
            let z = ev.at.to_zoned(tz.clone());
            let format = if z.year() == year { "%b %-d" } else { "%b %-d %Y" };
            z.strftime(format).to_string()
        })
        .collect();
    let day_width = days.iter().map(|d| d.width()).max().unwrap_or(0) + 2;
    let source_width = events.iter().map(|ev| ev.source.width()).max().unwrap_or(0) + 2;
    let label_width = (day_width + source_width).max(LABEL);
    let mut out = vec![Line::styled(s.title(e), bold()), Line::raw("")];
    for (ev, day) in events.iter().zip(&days) {
        let label = format!("{day:<day_width$}{}", ev.source);
        out.extend(table_row(&label, fg(t.quiet), label_width, &ev.text, Style::new(), width));
    }
    if let Some(text) = timeline::ships(s, e) {
        out.extend(table_row("Release", fg(t.quiet), label_width, &text, Style::new(), width));
    }
    out.push(Line::raw(""));
    out.push(Line::from(t.quiet(
        "Dates come from each tool. Review decisions and uplift approvals carry no date here.",
    )));
    out
}

fn idea_rows(out: &mut Vec<Line<'_>>, t: Theme, s: &Snapshot, i: worklens_core::IdeaIdx, scope: Scope, width: usize) {
    let idea = s.idea(i);
    let mut row = |label: &str, value: &str, style: Style| out.extend(table_row(label, fg(t.quiet), LABEL, value, style, width));
    let mut head = vec![idea.key.to_string(), idea.status.clone()];
    head.extend(idea.priority.clone());
    row("Idea", &head.join(" · "), Style::new());
    if let Some(w) = &idea.weekly_status {
        row("Weekly", w, Style::new());
    }
    let quarter = idea
        .quarter
        .as_ref()
        .filter(|q| !idea.target.as_ref().is_some_and(|t| t.contains(q.as_str())));
    let when: Vec<&String> = idea.target.iter().chain(quarter).collect();
    if !when.is_empty() {
        row(
            "Target",
            &when.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" · "),
            Style::new(),
        );
    }
    if let Some(theme) = &idea.theme {
        row("Theme", theme, Style::new());
    }
    let who: Vec<String> = [&idea.assignee, &idea.team].into_iter().flatten().cloned().collect();
    if !who.is_empty() {
        row("Owner", &who.join(" · "), Style::new());
    }
    if scope == Scope::Team || s.work_of(i).next().is_none() {
        return;
    }
    out.push(Line::raw(""));
    out.push(divider(t, "Your work under it", width));
    for w in s.work_of(i).map(|w| s.entry(w)) {
        out.extend(table_row(&s.id(w).to_string(), fg(t.quiet), LABEL, s.title(w), Style::new(), width));
        if w.placement.is_some() {
            out.extend(table_row("", fg(t.quiet), LABEL, &s.reason(w).to_string(), fg(t.quiet), width));
        }
    }
}

fn comments<'a>(app: &App, s: &'a Snapshot, e: &'a Entry, width: usize) -> Vec<Line<'a>> {
    let t = Theme::of(app.tone);
    let Some(b) = e.bug.map(|b| s.bug(b)) else {
        return vec![Line::from(t.quiet("No bug, so no comments."))];
    };
    let Some(thread) = s.raw.threads.get(&b.id) else {
        return vec![Line::from(t.quiet("Comments unavailable locally. B opens the bug in Bugzilla."))];
    };
    let mut out = Vec::new();
    let skipped = (thread.total as usize).saturating_sub(thread.comments.len());
    for (i, c) in thread.comments.iter().enumerate() {
        if i == 1 && skipped > 0 {
            out.push(Line::from(t.quiet(format!(
                "… {skipped} earlier {} (open the bug for all)",
                if skipped == 1 { "comment" } else { "comments" }
            ))));
            out.push(Line::raw(""));
        }
        let what = if i == 0 { "Description" } else { "Comment" };
        out.push(divider(
            t,
            &format!("{what} · {} · {}", short_email(&c.author), ago(c.time, app.now)),
            width,
        ));
        out.extend(c.text.lines().filter(|l| !l.trim().is_empty()).map(Line::raw));
        out.push(Line::raw(""));
    }
    out
}

fn links<'a>(t: Theme, s: &'a Snapshot, e: &'a Entry) -> Vec<Line<'a>> {
    let links = present::links(s, e);
    if links.is_empty() {
        return vec![Line::from(t.quiet("No links."))];
    }
    links
        .into_iter()
        .map(|l| {
            let k = match l.kind {
                LinkKind::Bug => "B",
                LinkKind::Jira => "J",
                LinkKind::Revision => "P",
                LinkKind::Lando => "L",
                LinkKind::Try => "T",
                LinkKind::Parent | LinkKind::Idea | LinkKind::Evidence => "",
            };
            Line::from(vec![
                Span::styled(format!("{k} "), bold()),
                Span::raw(fit(&l.label, 12)),
                t.quiet(l.url.trim_start_matches("https://").to_owned()),
            ])
        })
        .collect()
}

fn drift_lines(t: Theme, e: &Entry) -> Vec<Line<'_>> {
    if e.drift.is_empty() {
        return vec![Line::from(t.quiet("Bugzilla and Jira agree on this one."))];
    }
    let mut out = Vec::new();
    for d in &e.drift {
        let (mark, color) = if d.level == Level::Error { ("✗", t.error) } else { ("~", t.warn) };
        out.push(Line::styled(format!("{mark} {}", d.message), fg(color)));
        out.push(Line::from(t.quiet(format!("  why: {}", d.cause))));
        if let Some(fix) = &d.fix {
            out.push(Line::styled(
                format!("  Fix: {} (Enter for actions)", present::FixText(fix)),
                fg(t.accent),
            ));
        }
        out.push(Line::raw(""));
    }
    out
}

pub const MENU_NUMBERED: usize = 10;

fn menu_number(i: usize) -> &'static str {
    const NUMBERS: [&str; MENU_NUMBERED] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    NUMBERS.get(i).copied().unwrap_or(" ")
}

fn menu_lines(app: &App, actions: &[crate::app::ItemAction], width: u16) -> Vec<Vec<Line<'static>>> {
    let t = Theme::of(app.tone);
    const INDENT: usize = 4;
    const KEY: usize = 3;
    let label_width = (width as usize).saturating_sub(INDENT + KEY).max(1);
    actions
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let selected = i == app.menu_cursor;
            let mark = if selected { t.cursor() } else { " " };
            let quiet = if selected { Color::Reset } else { t.quiet };
            let parts = wrap(&a.label, label_width);
            parts
                .into_iter()
                .enumerate()
                .map(|(n, part)| {
                    let mut spans = if n == 0 {
                        vec![
                            Span::styled(mark, fg(t.accent)),
                            Span::styled(format!(" {} ", menu_number(i)), fg(quiet)),
                        ]
                    } else {
                        vec![Span::raw(" ".repeat(INDENT))]
                    };
                    let key = a.key.filter(|_| n == 0).map_or_else(String::new, String::from);
                    spans.push(Span::raw(fit(&part, label_width)));
                    spans.push(Span::styled(format!("{key:>KEY$}"), fg(quiet)));
                    let line = Line::from(spans);
                    if !selected {
                        line
                    } else if t.select.is_none() {
                        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                        Line::styled(text, t.selection())
                    } else {
                        line.style(t.selection())
                    }
                })
                .collect()
        })
        .collect()
}

fn menu_needs_body(app: &App, actions: &[crate::app::ItemAction], side: Rect) -> bool {
    let lines: usize = menu_lines(app, actions, side.width.saturating_sub(4)).iter().map(Vec::len).sum();
    lines + 3 > side.height as usize
}

fn menu_body(app: &App, actions: &[crate::app::ItemAction]) -> Rect {
    let area = Rect::new(0, 2, app.size.0, app.size.1.saturating_sub(3));
    let side = body_panes(app, area)[1];
    let menu = if menu_needs_body(app, actions, side) { area } else { side };
    Rect::new(menu.x + 2, menu.y + 1, menu.width.saturating_sub(4), menu.height.saturating_sub(3))
}

pub fn menu_scroll_max(app: &App, actions: &[crate::app::ItemAction]) -> u16 {
    let body = menu_body(app, actions);
    let lines: usize = menu_lines(app, actions, body.width).iter().map(Vec::len).sum();
    lines.saturating_sub(body.height as usize).min(u16::MAX as usize) as u16
}

fn draw_menu(app: &App, f: &mut Frame<'_>, area: Rect, actions: &[crate::app::ItemAction]) {
    f.render_widget(Clear, area);
    let title = app
        .snapshot
        .as_ref()
        .zip(app.selected())
        .map_or_else(String::new, |(s, e)| format!(" Actions · {} ", s.id(e)));
    let t = Theme::of(app.tone);
    let outer = block(t.accent).title(title).padding(Padding::horizontal(1));
    let inner = outer.inner(area);
    f.render_widget(outer, area);
    let [body, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
    let per_action = menu_lines(app, actions, body.width);
    let above: usize = per_action.iter().take(app.menu_cursor).map(Vec::len).sum();
    let lines: Vec<Line<'static>> = per_action.into_iter().flatten().collect();
    let total = lines.len();
    let scroll = (above + app.menu_scroll as usize).min(total.saturating_sub(body.height as usize));
    f.render_widget(Paragraph::new(lines).scroll((scroll.min(u16::MAX as usize) as u16, 0)), body);
    if total > body.height as usize {
        let mut line = hint_line(t, keys::MENU_SCROLL_HINTS);
        let at = format!("Action {} of {}{}", app.menu_cursor + 1, actions.len(), keys::SEP);
        line.spans.insert(0, t.quiet(at));
        f.render_widget(Paragraph::new(line), status);
    }
}

const HELP_MAX: usize = 84;

fn tag_grid(t: Theme, width: usize) -> Vec<Line<'static>> {
    const TAG: usize = 4;
    const CELL: usize = TAG + 14 + 1;
    let cols = (width / CELL).clamp(1, 4);
    let cell = width / cols;
    let rows = keys::LEGEND_LEN.div_ceil(cols);
    (0..rows)
        .map(|r| {
            let mut spans = Vec::new();
            for c in 0..cols {
                if let Some((tag, meaning)) = keys::legend(c * rows + r) {
                    spans.push(Span::styled(fit(tag, TAG), bold()));
                    spans.push(t.quiet(fit(meaning, cell.saturating_sub(TAG))));
                }
            }
            Line::from(spans)
        })
        .collect()
}

fn help_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let t = Theme::of(app.tone);
    let col = width / 2;
    let heading = |text: &str| Line::styled(text.to_owned(), bold());
    let mut left = vec![Line::from(vec![
        Span::styled("Keys", bold()),
        t.quiet(format!(" · vim too: {}", keys::vim_keys())),
    ])];
    for b in keys::BINDINGS.iter().filter(|b| !b.label.is_empty()) {
        left.extend(table_row(b.label, bold(), 11, b.help, fg(t.quiet), col.saturating_sub(1)));
    }
    let mut right = vec![heading("On the selected item")];
    let items = keys::ITEM_KEYS
        .iter()
        .map(|(k, d)| ((*k).to_owned(), (*d).to_owned()))
        .chain(app.custom_key_help());
    for (k, d) in items {
        right.extend(table_row(&k, bold(), 3, &d, fg(t.quiet), width - col));
    }
    let mut out = Vec::new();
    for i in 0..left.len().max(right.len()) {
        let mut spans = left.get(i).map_or_else(Vec::new, |l| l.spans.clone());
        let used: usize = spans.iter().map(Span::width).sum();
        spans.push(Span::raw(" ".repeat(col.saturating_sub(used))));
        if let Some(r) = right.get(i) {
            spans.extend(r.spans.iter().cloned());
        }
        out.push(Line::from(spans));
    }
    out.push(heading("Tags in the list"));
    out.extend(tag_grid(t, width));
    out
}

fn help_layout(app: &App, area: Rect) -> (Rect, Vec<Line<'static>>, usize) {
    let width = (area.width.saturating_sub(2) as usize).min(HELP_MAX) as u16;
    let x = area.x + (area.width - width) / 2;
    let lines = help_lines(app, width as usize);
    let top = u16::from(lines.len() < area.height as usize);
    let body = Rect::new(x, area.y + top, width, area.height - top);
    let max = lines.len().saturating_sub(body.height as usize);
    let max = if max > 0 { max + 1 } else { 0 };
    (body, lines, max)
}

pub fn help_scroll_max(app: &App) -> u16 {
    let area = Rect::new(0, 2, app.size.0, app.size.1.saturating_sub(3));
    help_layout(app, area).2.min(u16::MAX as usize) as u16
}

fn draw_help(app: &App, f: &mut Frame<'_>, area: Rect) {
    let t = Theme::of(app.tone);
    let (body, lines, max) = help_layout(app, area);
    if max == 0 {
        f.render_widget(Paragraph::new(lines), body);
        return;
    }
    let [content, hint] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(body);
    let total = lines.len();
    let scroll = (app.help_scroll as usize).min(max);
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), content);
    let line = scroll_hint(t, scroll, total, content.height as usize, keys::HELP_SCROLL_HINTS);
    f.render_widget(Paragraph::new(line), hint);
}

fn short_url(url: &str) -> String {
    let url = url.trim_start_matches("https://").trim_start_matches("http://");
    match url.strip_prefix("api.atlassian.com/ex/jira/").and_then(|rest| rest.split_once('/')) {
        Some((_, rest)) => format!("jira/{rest}"),
        None => url.to_owned(),
    }
}

fn draw_log(app: &App, f: &mut Frame<'_>, area: Rect) {
    let t = Theme::of(app.tone);
    let outer = block(t.rule).padding(Padding::horizontal(1));
    let inner = outer.inner(area);
    f.render_widget(outer, area);
    let calls = app.snapshot.as_ref().map_or(&[][..], |s| s.raw.calls.as_slice());
    let summary = if calls.is_empty() {
        "No requests recorded yet; the log starts with the next fetch.".to_owned()
    } else {
        let failed = calls.iter().filter(|c| c.error.is_some() || c.status.unwrap_or(0) >= 400).count();
        let mut hosts: Vec<&str> = calls.iter().filter_map(|c| c.url.split('/').nth(2)).collect();
        hosts.sort_unstable();
        hosts.dedup();
        let slowest = calls.iter().max_by_key(|c| c.ms).map_or_else(String::new, |c| {
            format!("; slowest {}ms ({})", c.ms, short_url(c.url.split('?').next().unwrap_or_default()))
        });
        format!(
            "{} requests to {} services, {}{slowest}",
            calls.len(),
            hosts.len(),
            if failed > 0 {
                format!("{failed} failed")
            } else {
                "none failed".into()
            }
        )
    };
    let tz = jiff::tz::TimeZone::system();
    let mut lines = vec![Line::styled("Activity", bold()), Line::from(t.quiet(summary))];
    for c in calls
        .iter()
        .rev()
        .skip(app.log_offset)
        .take((inner.height as usize).saturating_sub(2))
    {
        let bad = c.error.is_some() || c.status.unwrap_or(0) >= 400;
        lines.push(Line::from(vec![
            t.quiet(format!("{} ", c.at.to_zoned(tz.clone()).strftime("%H:%M:%S"))),
            Span::styled(format!("{:>5}ms ", c.ms), fg(if c.ms > 3000 { t.warn } else { t.quiet })),
            Span::styled(
                format!("{:<4}", c.status.map_or_else(|| "err".to_owned(), |x| x.to_string())),
                fg(if bad { t.error } else { t.quiet }),
            ),
            t.quiet(fit(&c.method, 5)),
            Span::styled(c.error.as_ref().map(|e| format!("{e} · ")).unwrap_or_default(), fg(t.error)),
            Span::raw(short_url(&c.url)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_setup(app: &App, f: &mut Frame<'_>, area: Rect) {
    let t = Theme::of(app.tone);
    let outer = block(t.rule).padding(Padding::horizontal(1));
    let body = outer.inner(area);
    f.render_widget(outer, area);
    let current = usize::from(app.page == Page::Settings);
    draw_box_tabs(t, f, &["Sources", "Settings"], current, area);
    match app.page {
        Page::Sources => draw_scrollable(t, source_lines(app, body.width as usize), app.pane_scroll, f, body),
        Page::Settings => {
            draw_scrollable(t, setting_lines(app, body.width as usize), app.pane_scroll, f, body);
            if let Some(toast) = &app.toast {
                let line = Line::styled(format!("{}   ", toast.text), if toast.error { fg(t.error) } else { fg(t.ok) });
                let w = (line.width() as u16).min(body.width);
                let [_, hint] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(body);
                let at = Rect::new(hint.x, hint.y, w, 1);
                f.render_widget(Clear, at);
                f.render_widget(Paragraph::new(line), at);
            }
        }
    }
}

pub fn sources_scroll_max(app: &App) -> u16 {
    let width = app.size.0.saturating_sub(4);
    Paragraph::new(source_lines(app, width as usize))
        .wrap(Wrap { trim: false })
        .line_count(width)
        .saturating_sub(app.size.1.saturating_sub(6) as usize)
        .min(u16::MAX as usize) as u16
}

const SETTING_LABEL: usize = 18;
const SETTING_VALUE: usize = 13;
const CHANGED: &str = "changed";

fn setting_value(app: &App, setting: Setting) -> String {
    let value = app.prefs.value(setting);
    match setting {
        Setting::Theme if app.prefs.theme == ThemeName::Auto => match app.tone {
            Tone::Dark => "auto (dark)".to_owned(),
            Tone::Light => "auto (light)".to_owned(),
            _ => value,
        },
        Setting::Agent if app.prefs.agent.is_empty() => match &app.opts.agent {
            Some(a) => format!("auto ({})", a.name),
            None => "auto (none found)".to_owned(),
        },
        _ => value,
    }
}

fn setting_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let t = Theme::of(app.tone);
    let current = app.current_setting();
    let mut lines = Vec::new();
    for (g, (heading, group)) in settings::groups().into_iter().enumerate() {
        if g > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(heading, bold()));
        for setting in group {
            let selected = setting == current;
            let marker = if selected { t.cursor() } else { " " };
            let mut spans = vec![Span::styled(marker, fg(t.accent)), Span::raw(fit(setting.label(), SETTING_LABEL))];
            let editing = app.editing.as_ref().filter(|_| selected);
            let changed = app.prefs.changed(setting);
            let room = width.saturating_sub(1 + SETTING_LABEL);
            if let Some(text) = editing {
                spans.push(Span::styled(format!("{}▏", input_tail(text, room.saturating_sub(2))), fg(t.accent)));
            } else {
                let value = setting_value(app, setting);
                let arrows = selected && setting.kind() == Kind::Toggle;
                let value_cells = (value.width() + if arrows { 4 } else { 0 }).max(SETTING_VALUE - 1) + 1;
                let tail = if changed { CHANGED.width() + 1 } else { 0 };
                let note_room = room.saturating_sub(value_cells + tail);
                let value_style = if setting.kind() == Kind::Fixed { fg(t.quiet) } else { Style::new() };
                let value = if arrows { format!("‹ {value} ›") } else { value };
                spans.push(Span::styled(fit(&value, value_cells.min(room)), value_style));
                spans.push(t.quiet(fit(setting.note(&app.prefs), note_room)));
                if changed && room > value_cells + tail {
                    spans.push(t.quiet(format!(" {CHANGED}")));
                }
            }
            let mut line = Line::from(spans);
            if selected && editing.is_none() {
                if t.select.is_none() {
                    for span in &mut line.spans {
                        span.style = span.style.fg(t.accent);
                    }
                }
                line = line.style(t.selection());
            }
            lines.push(line);
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(
        t.quiet("Credentials, Jira statuses and custom keys: ~/.config/worklens.json"),
    ));
    lines
}

fn hanging(head: &[Span<'static>], indent: usize, text: &str, style: Style, width: usize) -> Vec<Line<'static>> {
    wrap(text, width.saturating_sub(indent).max(8))
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let mut spans = if i == 0 {
                head.to_vec()
            } else {
                vec![Span::raw(" ".repeat(indent))]
            };
            spans.push(Span::styled(part, style));
            Line::from(spans)
        })
        .collect()
}

fn source_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let t = Theme::of(app.tone);
    let mut lines = Vec::new();
    if let Some(e) = &app.action_error {
        lines.push(Line::styled(format!("Last action failed: {e}"), fg(t.error)));
    }
    if let Some(e) = &app.error {
        lines.push(Line::styled(e.clone(), fg(t.error)));
        lines.push(Line::raw(""));
    }
    if let Some(s) = &app.snapshot {
        for error in s.raw.failures() {
            lines.push(Line::styled(format!("! {}: {}", error.source, error.message), fg(t.error)));
        }
    }
    match &app.checks {
        None => lines.push(Line::from(vec![
            Span::styled(format!("{} ", SPINNER[app.frame % SPINNER.len()]), fg(t.accent)),
            t.quiet("Reading your sources and tokens…"),
        ])),
        Some(checks) => {
            let spinner = SPINNER[app.frame % SPINNER.len()];
            let name_width = checks.iter().map(|c| c.name.width()).max().unwrap_or(0) + 1;
            let indent = 2 + name_width;
            for c in checks {
                let (mark, color) = match (c.pending, c.ok, c.required) {
                    (true, _, _) => (spinner.to_string(), t.accent),
                    (false, true, _) => ("✓".to_owned(), t.ok),
                    (false, false, true) => ("✗".to_owned(), t.error),
                    (false, false, false) => ("~".to_owned(), t.warn),
                };
                let head = vec![
                    Span::styled(format!("{mark} "), fg(color)),
                    Span::raw(format!("{:<name_width$}", c.name)),
                ];
                let style = if c.pending { t.quiet("").style } else { Style::default() };
                lines.extend(hanging(&head, indent, &c.detail, style, width));
                if let (false, false, Some(fix)) = (c.pending, c.ok, &c.fix) {
                    let head = vec![Span::raw(" ".repeat(indent))];
                    lines.extend(hanging(&head, indent, &format!("fix: {fix}"), fg(t.accent), width));
                }
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(
                t.quiet("✗ needed · ~ optional: worklens works without it, with less in the view"),
            ));
        }
    }
    lines
}

fn draw_scrollable(t: Theme, lines: Vec<Line<'_>>, offset: u16, f: &mut Frame<'_>, area: Rect) {
    let [body, hint] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = paragraph.line_count(body.width);
    let max = total.saturating_sub(body.height as usize);
    let scroll = (offset as usize).min(max);
    f.render_widget(paragraph.scroll((scroll.min(u16::MAX as usize) as u16, 0)), body);
    if max > 0 {
        let line = scroll_hint(t, scroll, total, body.height as usize, keys::DETAIL_SCROLL_HINTS);
        f.render_widget(Paragraph::new(line).alignment(Alignment::Right), hint);
    }
}

fn since_line(t: Theme, at: Timestamp, since: &timeline::Since, now: Timestamp, width: usize) -> Line<'static> {
    let tail = [t.quiet(keys::SEP), Span::raw("?"), t.quiet(" help")];
    let room = width.saturating_sub(tail.iter().map(Span::width).sum());
    if since.changed + since.new == 0 {
        let mut spans = vec![Span::raw(
            fit(&format!("Nothing new since you looked, {}", ago(at, now)), room)
                .trim_end()
                .to_owned(),
        )];
        spans.extend(tail);
        return Line::from(spans);
    }
    let mut counts = Vec::new();
    if since.changed > 0 {
        counts.push(format!("{} changed", since.changed));
    }
    if since.new > 0 {
        counts.push(format!("{} new", since.new));
    }
    let head = format!("Since you looked, {}: {}", ago(at, now), counts.join(", "));
    let mut spans = vec![Span::styled(fit(&head, room).trim_end().to_owned(), fg(t.accent))];
    let mut used = head.width();
    for (n, h) in since.highlights.iter().enumerate() {
        let left = room.saturating_sub(used + 3);
        let text = if h.width() <= left {
            h.clone()
        } else if n == 0 && left >= 12 {
            fit(h, left).trim_end().to_owned()
        } else {
            break;
        };
        used += 3 + text.width();
        spans.push(Span::styled(" · ", fg(t.rule)));
        spans.push(Span::raw(text));
    }
    spans.extend(tail);
    Line::from(spans)
}

fn draw_status(app: &App, f: &mut Frame<'_>, area: Rect) {
    let t = Theme::of(app.tone);
    let area = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    });
    let line = if let Some(toast) = &app.toast
        && (app.mode == Mode::List || (app.mode == Mode::Setup && app.page == Page::Sources))
    {
        Line::from(vec![
            Span::styled(
                fit(&toast.text, area.width.saturating_sub(9) as usize).trim_end().to_owned(),
                if toast.error { fg(t.error) } else { Style::new() },
            ),
            t.quiet(keys::SEP),
            Span::raw("?"),
            t.quiet(" help"),
        ])
    } else if let (Some((at, since)), Mode::List) = (&app.since, app.mode) {
        since_line(t, *at, since, app.now, area.width as usize)
    } else {
        match app.mode {
            Mode::List if app.snapshot.is_none() => hint_line(t, keys::NO_CACHE_HINTS),
            Mode::List => hint_line(t, &keys::list_hint_pairs()),
            Mode::Menu => hint_line(t, keys::MENU_HINTS),
            Mode::Filter => hint_line(t, keys::FILTER_HINTS),
            Mode::Help => hint_line(t, keys::HELP_HINTS),
            Mode::Setup if app.editing.is_some() => hint_line(t, keys::EDIT_HINTS),
            Mode::Setup if app.page == Page::Settings => {
                let setting = app.current_setting();
                let enter = app.prefs.enter_hint(setting);
                let mut pairs = vec![("↑↓", "move"), ("Enter", enter.as_str())];
                if setting.kind() == Kind::Toggle {
                    pairs.push(("←→", "choose"));
                }
                pairs.extend_from_slice(keys::SETTINGS_HINTS);
                hint_line(t, &pairs)
            }
            Mode::Setup => hint_line(t, keys::SETUP_HINTS),
            Mode::Log => hint_line(t, keys::LOG_HINTS),
            Mode::Snooze => {
                let id = app
                    .snapshot
                    .as_ref()
                    .zip(app.selected())
                    .map(|(s, e)| s.id(e).to_string())
                    .unwrap_or_default();
                let mut line = hint_line(t, keys::EDIT_HINTS);
                let text = input_tail(&app.snooze_text, area.width.saturating_sub(56) as usize);
                line.spans.insert(0, t.quiet(format!("Snooze {id} locally: {text}▏{}", keys::SEP)));
                line
            }
        }
    };
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_pads_and_cuts() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("x", 0), "");
        assert_eq!(fit("日本語", 5), "日本…");
        assert_eq!(fit("e\u{301}abc", 3), "e\u{301}a…");
        assert_eq!(fit("日", 3), "日 ");
        assert_eq!(fit("👩‍💻a", 2), "… ");
    }

    #[test]
    fn ages_read_naturally() {
        let now: Timestamp = "2026-11-02T12:00:00Z".parse().unwrap();
        assert_eq!(ago(now, now), "just now");
        assert_eq!(ago("2026-11-02T11:55:00Z".parse().unwrap(), now), "5m ago");
        assert_eq!(ago("2026-10-31T12:00:00Z".parse().unwrap(), now), "2d ago");
    }

    #[test]
    fn jira_api_urls_are_shortened() {
        assert_eq!(
            short_url("https://api.atlassian.com/ex/jira/abc-123/rest/api/3/myself"),
            "jira/rest/api/3/myself"
        );
        assert_eq!(short_url("https://bugzilla.mozilla.org/rest/bug"), "bugzilla.mozilla.org/rest/bug");
    }
}
