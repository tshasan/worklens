# TUI design

Rules for the TUI. Code lives in `crates/tui/src`: layout and styles in
`view.rs`, state in `app.rs`, list keys in `keys.rs`.

## Layout

- 80x24 must work. Below that width, stack the list above the detail; above
  it, put them side by side. Keep the selection when the terminal resizes.
- The screen has four parts:
  - a header: sprint, source health and cache age;
  - section tabs;
  - the list and the detail;
  - a footer of keys for the current context.
- When the header is short of room, drop the merge countdown first, then
  shorten source health to "Sources ✓" or the sources that aren't set up.
- Row order: activity, age, priority, tag, try/drift status, ID, title. Align
  columns and reserve slots for optional marks so titles don't move. Priority
  (`!` critical, `*` important) has its own column, apart from the tag.
- A list at least 96 cells wide spells tags out ("asked of you", not `ni?`)
  and, when tall enough, labels its columns.
- Measure width in terminal cells, not bytes or `char`s. Truncate titles in
  the list with `…`; wrap text in the detail.
- Detail order: title, reason, next step, then facts and comments. On a
  wide screen at least 40 rows tall, the timeline sits inside the Overview
  and loses its own tab.
- Fold release channels with nothing known into one "Unknown" row. "not
  set" is a fact and keeps its row.
- Show commit hashes as their first 12 characters.
- Put a blank line between subjects, not between facts. Prefer spacing to
  borders.

## Color

Colors come from `Theme` in `view.rs`. The background is detected over OSC 11
in `crates/worklens/src/tone.rs`. Greys use the 256-color ramp; accent and
status colors use ANSI names so they follow the user's theme.

| Role | Dark | Light | Use |
|---|---|---|---|
| Primary | default fg | default fg | titles, values, reasons |
| `quiet` | grey 246 | grey 242 | ages, tags, IDs, labels |
| `rule` | grey 240 | grey 250 | borders |
| Accent | cyan, bold | cyan, bold | current tab, cursor, next step |
| Selected row | grey 237 band, `▌` | grey 254 band, `▌` | |
| Good / warning / error | green / amber / red | same | status only |

- Use hue for exceptions. A normal case, such as a 200 response or a
  needinfo tag, stays uncolored.
- Pair every color with a word or symbol. A stale pass must look different
  from a current one without color.
- Going over the sprint budget is a warning, not an error.
- With no known background (`NO_COLOR`, `plain`, unknown), selection is
  reverse video with `>`.
- Don't use dim text for reasons, IDs, errors or key hints.
- Don't require icon fonts. Animate only the busy indicator.

## Tabs

The first tab is **All**: every section that needs you (On you, Land,
Returned, On others, Sprint, Drift, Local, Ideas) in one list, each item once,
under a heading for the first of those sections that holds it. Backlog and
Landed stay in their own tabs; All's border says how many they hold.

Tabs use the short names everywhere (`On you`, `On others`, `Backlog`); the
full title is the heading inside All. When tabs overflow, say how many are
hidden: `‹ 2 more`, `3 more ›`. While filtering, tabs keep their places and
show the matches in each.

Section and detail tabs look the same:
- names three cells apart;
- the current tab in the accent, with a heavy `━` under it, drawn in the top
  border of the box the tabs switch (the names sit on the row above it);
- the key that switches them in view.

## Text

- Numbers carry units: "6 of 10 pts", "13 days left", "Updated 3m ago".
- Actions name their target: "Open bug 1995008 in Bugzilla", "Copy brief".
- Feedback names the result: "Copied brief for D123". Report success only
  after the action has finished.

## Keys

- Arrows move, `Enter` opens the next step (what `o` opens), `Space` opens
  the actions popup, `Esc` backs out of a popup, prompt or filter. `Esc`
  never quits; `q` does.
- `←` `→` switch sections, `Tab` switches detail tabs.
- Item keys read verb then noun, as in vim. `g` goes: `gb` Bugzilla, `gj`
  Jira, `gp` Phabricator, `gl` Lando, `gt` try push, `ga` landing push, `g1`
  and on for other links. `y` copies: `yy` the ID, `yb` the brief, `yr` the
  review prompt, `yf` the drift fix. `gg` and `G` go to the top and bottom.
- Single letters act on the item: `d` done, `z` snooze, `c` coding agent,
  `C` check out, and the user's custom keys. `g` and `y` are reserved so
  custom keys can't shadow them.
- One key per action, no aliases. Add a new action as a row in `ITEM_KEYS`
  under its group; prefer a chord under `g` or `y` to a new letter.
- `u` undoes the last done or snooze; the confirmation says so.
- `g` or `y` alone opens the popup with only that group; the next key runs
  an action, and `Esc` or any other key cancels. `Space` shows every group
  with full chords (`g b`), and `g` or `y` there narrows it.
- The popup is a cheat sheet: the key on the left of every row (`⏎` for the
  next step), rows under Go, Copy and Item headings. It leaves out actions
  that can't run (no coding agent found).
- The popup sits under the selected row, sized to its rows, so the detail
  stays readable. Only a popup taller than the body covers it.
- Keys, footer, popup and help all come from `BINDINGS` and `ITEM_KEYS`.
  Help must fit 80x24.
- The footer fills by rank as width allows: at 80 columns the essentials,
  `g go` and `y copy` among them; on a full screen filter, done, snooze,
  refresh and sources too.
- Footers and hints read `key label · key label`: keys in the default fg,
  labels and separators quiet. Scroll hints give the range with its unit and
  the keys: "1–9 of 13 lines · PgUp PgDn scroll".
- Confirmations start with `✓`, errors with `!`.
- Typing in a prompt (filter, snooze) edits the prompt, not item shortcuts,
  `g` and `y` included.

## States

Show the cache at once and keep it usable during a refresh. Never replace the
queue with a loading screen.

| State | Show |
|---|---|
| Empty section | "Nothing waiting on you" |
| Filter with no matches | the query, "No matches", how to clear it, and the next tab with matches |
| Everything hidden | that items are done or snoozed, and that `Z` shows them |
| Source failing | which source, and that `s` explains it. Don't imply there's no work |
| First run, no cache | what's being fetched, or one line saying `r` fetches; the detail says which sources need setting up |

## Before shipping a UI change

- At 80x24 and full screen, find the first item, read why it's there, open
  its actions with `Space` and close with `Esc`. Press `g`, then `y`, and
  read each popup; run one chord; check `Esc` in the list doesn't quit.
- Filter to a match, then to no match, then clear the filter. Mark an item
  done and undo it with `u`.
- Refresh while reading an item. Try a failing source, no cache and an empty
  queue.
- Check light and dark terminals, and with `NO_COLOR`.
