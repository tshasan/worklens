# Lands-in for agents

Status: implemented; boundary and interface verification required. Source: SPEC.md.

## Revised scope

The shared core calculation and additive output already exist. Verify them
rather than rebuilding them. Cover before/on/after merge, missing and stale
calendar data, and TUI/full JSON/compact/MCP agreement. Do not extrapolate an
old calendar indefinitely or imply confirmed build inclusion. SPEC.md's
release-evidence requirements override unconditional calendar claims below.
No new fields or settings are required unless a demonstrated correctness gap
needs them.

## Original design reference

The following describes the original proposal. It is subordinate to the
revised scope and is not an instruction to expand the implementation.

## Problem Statement

The TUI tells me "Lands in 162 if it lands by Nov 10" on accepted revisions,
but that text is built in the view. `--json`, `--compact` and MCP don't carry
it. Near merge day, "which release does this land in?" is the question agents
get asked most, and without the field they redo the calendar math and get it
wrong, usually by one version on merge day.

## Solution

The lands-in calculation moves into core, next to the train-position logic
Landed uses. Every accepted entry in JSON, compact output and MCP carries a
`landsIn` object: the version it lands in, the date it must land by to make
that version, and the version it lands in after that date. The TUI reads the
same value, so the screen and agents always agree.

## User Stories

1. As an agent, I want `landsIn.version` on accepted entries, so that I can say which release a patch lands in without calendar math.
2. As an agent, I want `landsIn.ifBy`, so that I can say "if it lands by Nov 10".
3. As an agent, I want `landsIn.after`, so that I can say what happens if it misses the merge.
4. As an agent, I want the field in `--json`, `--compact` and MCP alike, so that every interface gives the same answer.
5. As an agent, I want the field absent rather than wrong when train data is missing, so that I don't repeat a bad guess.
6. As an agent, I want the field added without renaming anything in `worklens/2`, so that existing agents keep working.
7. As a Firefox engineer, I want the TUI's "Lands in" line unchanged, so that nothing I rely on moves.
8. As a Firefox engineer, I want the TUI and JSON computed by the same function, so that they never disagree.
9. As a Firefox engineer, I want merge day itself handled correctly, so that the answer is right on the day it matters most.
10. As a skill author, I want the worklens skill to document `landsIn`, so that Claude Code uses it instead of computing.
11. As a maintainer, I want the calculation pure and taking `now`, so that it is testable at any date.

## Implementation Decisions

- Core gains a lands-in function taking the trains and `now`, returning version, if-by date and after-version, or nothing when trains are unknown. It lives with the train-position function from Landed (spec 03).
- The snapshot carries the result on accepted entries; the view's local calculation is deleted and reads it.
- JSON: additive `landsIn: { version, ifBy, after }` on accepted entries under schema `worklens/2`, emitted in full, compact and MCP output. `ifBy` is a date string in the same format as other dates in the schema.
- Skill: document the field and say to prefer it over computing.

## Testing Decisions

- A good test fixes `now` and the train calendar and checks the field an agent reads.
- Replay seam: a replay with `product-details.json` and `now` on merge day and the day after; `worklens land --json` must show `landsIn` with values worked out by hand from the fixture's calendar.
- Core unit tests at the boundaries: the day before merge, merge day, the day after, and missing trains (field absent).
- TUI render test: the "Lands in" line is unchanged on the existing fixture.
- Prior art: the replay tests in the worklens crate, `tui/tests/render.rs`.
- Seen red: shift the boundary by one day in the source; the merge-day test must fail.

## Out of Scope

- Lands-in for revisions that are not accepted.
- Uplift timing.

## Further Notes

- Builds after Landed because both touch the same train code.
