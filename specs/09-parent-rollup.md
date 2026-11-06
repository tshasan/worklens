# Parent rollup

Status: deferred; do not implement in this assignment. Source: SPEC.md.

## Decision

No grouping toggle, nested parent hierarchy, persistent grouping state,
parent progress counts or additional meta-bug fetching.

Reconsider only after daily use identifies a recurring decision the current
tool cannot help with. The original proposal below is retained for context,
not as an agent task. SPEC.md supersedes its implementation instructions.

## Archived proposal

## Problem Statement

Tying daily bugs to what they serve is why worklens reads Jira at all, but
nothing groups my items under their parent. Worklens loads Jira parents and
ideas and flags idea drift, and the Ideas section lists ideas. Firefox teams
often track work as `[meta]` bugs rather than epics, and worklens ignores
those, though it already fetches bug dependencies. So I can't see that three
of my tickets feed one epic due next week, or that a meta bug has one open
child left and it's mine.

## Solution

A grouping toggle in the list groups items under their parent: the Jira
epic, then the epic's idea; or the nearest `[meta]` bug my bug blocks. Each
group line shows the key or bug ID, title, stated target and status, open
children, and any existing drift. Items with no parent go under "No parent".
There is no completion percentage. JSON carries `epic`, `meta` and `idea`
per entry.

## User Stories

1. As a Firefox engineer, I want to toggle grouping by parent, so that I see what larger work each item is for.
2. As a Firefox engineer, I want Jira tickets grouped under their epic, so that I see which tickets feed one epic.
3. As a Firefox engineer, I want epics grouped under their idea, so that I see the product goal behind them.
4. As a Firefox engineer, I want my bugs grouped under the nearest `[meta]` bug they block, so that Bugzilla-tracked work rolls up too.
5. As a Firefox engineer, I want the group line to show the parent's stated target and status, so that I know whether it's on track.
6. As a Firefox engineer, I want the group line to show open children, including other people's, so that I see what's left.
7. As a Firefox engineer, I want to see when the last open child of a meta bug is mine, so that I know I'm the blocker.
8. As a Firefox engineer, I want the parent's existing drift on the group line, so that disagreements show where they matter.
9. As a Firefox engineer, I want no completion percentage, so that worklens doesn't invent a measure no tool states.
10. As a Firefox engineer, I want items without a parent under "No parent", so that nothing disappears when grouped.
11. As a Firefox engineer, I want the toggle in the actions menu or on an existing key, so that the help still fits 80x24.
12. As a Firefox engineer, I want the toggle to remember its state, so that I don't re-toggle every launch.
13. As a Firefox engineer, I want ideas to stay read-only context, so that worklens doesn't suggest editing them.
14. As a Firefox engineer, I want a bug with both an epic and a meta parent grouped under the epic, so that each item appears once.
15. As an agent using `--json` or MCP, I want `epic`, `meta` and `idea` keys on each entry, so that I can answer "what is this for?".
16. As a Firefox engineer, I want grouping to fit the 16 ms frame budget with 1500 items, so that toggling stays instant.

## Implementation Decisions

- Grouping is a view toggle, not a section. Core computes each entry's parent chain; the TUI groups by it.
- Parent order: Jira epic, then the epic's idea. For bugs, the nearest `[meta]` bug reached through `blocks` (shortest path, ties by lowest bug ID). An item with an epic uses the epic.
- Group line: key or bug ID, title, stated target and status, open children count, existing drift.
- No new fetches for epics and ideas. Meta bug titles and statuses may need one batched Bugzilla lookup in sources, with a fixture test and new Raw fields under `#[serde(default)]`.
- TUI: the toggle lives in the actions menu or an existing binding, a `BINDINGS` row if a key is added; settings persist it. Follow `AESTHETICS.md` for group lines inside the existing bordered panes.
- JSON: additive `epic`, `meta`, `idea` per entry under `worklens/2`. Update the skill.
- `busy.json` gains two epics under one idea, a meta bug with two of my children and one of someone else's, and an orphan ticket.

## Testing Decisions

- A good test builds the snapshot from the busy fixture and checks the grouped shape a user sees.
- `build_snapshot` seam: parent keys per entry match the fixture's links; the meta bug shows three open children; the orphan has no parent; the item with both parents is under its epic.
- TUI render test: the grouped view of `busy.json` shows two epics under one idea, the meta group and "No parent", at 80x24 and wider.
- Frame budget test: grouping on with 1500 items fits 16 ms.
- TUI robust: random key sequences include the toggle.
- Prior art: `core/tests/ideas.rs`, `tui/tests/render.rs`, `tui/tests/frame_budget.rs`.

## Out of Scope

- Completion percentages or burndown.
- Editing epics, ideas or meta bugs.
- Parents more than one level above a meta bug.

## Further Notes

- See the memory note on Jira glue: ideas are context and read-only.
