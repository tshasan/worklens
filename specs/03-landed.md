# Landed

Status: implemented; correctness and presentation audit required. Source: SPEC.md.

## Revised scope

Keep Landed as quiet reference material after actionable sections. No
additional release-monitoring features or warning color for normal completed
work. Audit fixed-version evidence and calendar freshness. A status flag,
expected train position and confirmed build inclusion are different facts;
state their limits. A version below Beta is not sufficient proof of shipping.
Check release day separately from merge day, and missing/stale source data.
SPEC.md overrides any unconditional shipping claim below. Backlog is removed
from TUI navigation under revised spec 06; do not implement a 12-tab layout.
The weekly-summary dependency is withdrawn under revised spec 07.

## Original design reference

The following describes the original proposal. It is subordinate to the
revised scope and is not an instruction to expand the implementation.

## Problem Statement

QA, reporters and release management ask "is the fix in Beta yet?", and the
answer decides whether to request uplift. Worklens already fetches the bugs I
fixed in the last 30 days, then drops them unless they need uplift or have
drift. The release table only shows on items that stay in a list. So to
answer the question I open each bug and read its status flags against the
release calendar by hand.

## Solution

A new Landed section lists what I fixed in the last 30 days, newest first.
Each entry says which version it was fixed in and where that is on the
train: on autoland, in Nightly with the date it merges to Beta, in Beta with
the date it ships, or shipped. When a bug has no status flag to read, the
reason says so rather than guessing from dates. Uplift and status-flag
problems keep their existing rules.

## User Stories

1. As a Firefox engineer, I want a Landed section listing bugs I fixed in the last 30 days, so that I can answer "where is my fix?" in one place.
2. As a Firefox engineer, I want Landed sorted newest first, so that the fixes people are asking about are at the top.
3. As a Firefox engineer, I want "Fixed in 161, in Nightly; merges to Beta Nov 10", so that I know when it reaches Beta without checking the calendar.
4. As a Firefox engineer, I want "Fixed in 160, in Beta; ships Dec 1", so that I can tell a reporter when they'll get it.
5. As a Firefox engineer, I want "Fixed in 159, shipped", so that I know it's out.
6. As a Firefox engineer, I want "On autoland, not yet in Nightly" for a landing that hasn't merged, so that I don't tell QA to test a Nightly that doesn't have it.
7. As a Firefox engineer, I want the fixed version taken from the lowest `cf_status_firefoxN = fixed`, so that the answer matches what release management reads.
8. As a Firefox engineer, I want a bug with no fixed status flag to say "no status flag set", so that worklens never guesses a version from dates.
9. As a Firefox engineer, I want a bug that needs uplift to stay in Uplift, so that the actionable rule wins over the informational one.
10. As a Firefox engineer, I want a bug with drift to stay in Drift, so that a disagreement isn't hidden in Landed.
11. As a Firefox engineer, I want the Landed tag `ld` in the help legend, so that I can read the list.
12. As a Firefox engineer, I want Landed entries to show the existing release table when selected, so that I see the per-channel status.
13. As a Firefox engineer, I want the section strip still to fit 80 columns with Landed and Backlog added, so that the TUI works in a standard terminal.
14. As a Firefox engineer, I want Landed and Backlog to default to `auto` visibility if the strip doesn't fit, so that the layout never breaks.
15. As a Firefox engineer, I want `worklens landed` on the CLI, so that I can paste the list into a reply.
16. As an agent using `--json` or MCP sections, I want Landed entries with the fixed version and train position, so that I can answer "is it in Beta?" without calendar math.
17. As a Firefox engineer, I want an entry to leave Landed after 30 days, so that the list stays short.
18. As a Firefox engineer, I want the train dates taken from product-details, so that the dates are Mozilla's, not worklens's.
19. As the week summary (spec 07), I want to reuse Landed's entries, so that "what I landed" is computed once.

## Implementation Decisions

- Core: new `Section::Landed`, added to `Section::ALL`. New `Reason::Landed` with a `RULES` row, `Facets` tag `ld`, kept when landed, sorted newest first by fixed time. Its `next_step` opens the bug. Its text in `ReasonText`.
- Placement: landed-only entries that no higher rule takes go to Landed instead of being removed. Uplift, status-flag and drift rules keep priority.
- The fixed version is the lowest `N` where `cf_status_firefoxN` is `fixed`. No such flag gives a "no status flag" reason.
- Train position is computed in core from the fixed version and `Trains` (`nightly`, `next_merge`, `next_release`) with `now` passed in: version above Beta is in Nightly; equal to Beta is in Beta; at or below Release is shipped. The landing scan decides "on autoland, not yet in Nightly".
- The train-position function is shared with Lands-in (spec 04); write it once in core.
- CLI: a `landed` subcommand matching the other section subcommands. MCP sections include Landed. JSON entries gain additive `fixedIn` and `train` fields under `worklens/2`. Update the skill.
- TUI: a tab for Landed in the section strip. The help legend picks up `ld` from `rules::TAGS`.
- No new fetch: the 30-day `fixed_by` query already runs.
- `busy.json` gains one fixed bug per train position and one with no status flag.

## Testing Decisions

- A good test sets the status flags and the train calendar in a fixture and checks the reason a user reads. Expected versions and dates come from the fixture, not from the function under test.
- `build_snapshot` seam: one bug per train position, one with no flag, one that needs uplift (stays in Uplift), one with drift (stays in Drift), one 31 days old (gone). Assert section and reason text.
- Edge: `now` on merge day and the day after, so the Nightly/Beta boundary is pinned.
- TUI render test: section strip with all 12 tabs at 80 columns; if it fails the defaults switch to `auto` and the test asserts that instead.
- CLI test: `worklens landed --cached` against `busy.json` lists the entries.
- Prior art: `core/tests/uplift.rs` (status flags), `core/tests/busy.rs`, `tui/tests/render.rs`, `worklens/tests/cli.rs`.
- Seen red: break the lowest-N rule and the Beta boundary; mutants in the train-position function must die.

## Out of Scope

- Uplift deadlines; they need a release-schedule source beyond product-details.
- Guessing a fixed version from landing dates when no flag is set.
- History beyond 30 days.

## Further Notes

- The strip-width question is shared with Backlog (spec 06); decide the default visibility once both exist.
