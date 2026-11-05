# TUI design

Rules for the TUI. Code lives in `crates/tui/src`: layout and styles in
`view.rs`, state in `app.rs`, list keys in `keys.rs`.

## Layout

- 80x24 must work. Below that width, stack the list above the detail; above
  it, put them side by side. Keep the selection when the terminal resizes.
- The screen has four parts:
  - a header: sprint, cache age and source health;
  - section tabs;
  - the list and the detail;
  - a footer of keys for the current context.
- Row order: activity, age, tag, try/drift status, ID, title. Align columns
  and reserve slots for optional marks so titles don't move.
- Measure width in terminal cells, not bytes or `char`s. Truncate titles in
  the list with `…`; wrap text in the detail.
- Detail order: title, reason, next step, then facts and comments.
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

- Arrows move, `Enter` opens actions, `Esc` backs out.
- `←` `→` switch sections, `Tab` switches detail tabs.
- Add new actions to the actions menu before giving them a letter.
- Keys, footer, menu and help all come from `BINDINGS`. Help must fit 80x24.
- Footers and hints read `key label · key label`: keys in the default fg,
  labels and separators quiet. Scroll hints give the range with its unit and
  the keys: "1–9 of 13 lines · PgUp PgDn scroll".
- Typing in a prompt (filter, snooze) edits the prompt, not item shortcuts.

## States

Show the cache at once and keep it usable during a refresh. Never replace the
queue with a loading screen.

| State | Show |
|---|---|
| Empty section | "Nothing waiting on you" |
| Filter with no matches | the query, "No matches", how to clear it |
| Everything hidden | that items are done or snoozed, and that `Z` shows them |
| Source failing | which source, and that `s` explains it. Don't imply there's no work |
| First run, no cache | what's being fetched |

## Before shipping a UI change

- At 80x24, find the first item, read why it's there, open its actions with
  `Enter` and close with `Esc`.
- Filter to a match, then to no match, then clear the filter. Mark an item
  done and restore it.
- Refresh while reading an item. Try a failing source, no cache and an empty
  queue.
- Check light and dark terminals, and with `NO_COLOR`.
