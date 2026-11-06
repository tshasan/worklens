# Stalled

Status: implemented; factual wording and threshold audit required. Source: SPEC.md.

## Revised scope

Retain existing sections and the existing stale mechanism. Audit each new
case against its actual timestamp and a relevant, verified policy source.
Review-request age is not the same as time since last revision modification;
a bug update is not proof of patch progress. Name the measured event in the
reason. Do not assert a rebase is needed, a reviewer is blocked, or a user is
neglecting work from elapsed time alone. Do not introduce additional priority
promotion or automatic next steps based only on the new inactivity signals.
Existing review-follow-up behavior should not be expanded by this work.
If a new stale threshold lacks applicable provenance, withdraw that new case
rather than inventing a policy. Missing/future timestamps and failed sources
must not manufacture staleness. SPEC.md overrides the original motivating
claims of blockage and required action below.

## Original design reference

The following describes the original proposal. It is subordinate to the
revised scope and is not an instruction to expand the implementation.

## Problem Statement

Worklens marks only accepted revisions and asked flags as stale, after seven
days. A revision left in review for a month, a Jira ticket "in progress" for
six weeks, or an assigned bug with an open revision untouched for a month all
look healthy. The revision needs a rebase, the ticket misleads the team
board, and I usually notice at sprint review. Drift doesn't help: it catches
tools disagreeing, not work going quiet.

## Solution

The existing stale mechanism covers three more cases, with thresholds taken
from BugBot's inactivity rules. A stale item says how long it has been
quiet: "D123 in review, no activity for 21 days". An item with no timestamp
is unknown, never stale.

## User Stories

1. As a Firefox engineer, I want my revision in review with no activity past the threshold marked stale, so that I rebase or ping before it rots.
2. As a Firefox engineer, I want my Jira ticket in progress with no update past the threshold marked stale, so that the board stays honest.
3. As a Firefox engineer, I want my assigned bug with an open revision and no activity marked stale, so that half-finished work surfaces.
4. As a Firefox engineer, I want "D123 in review, no activity for 21 days", so that the reason states a fact I can check.
5. As a Firefox engineer, I want "PROJ-12 in progress, no update for 30 days", so that I know which ticket to update.
6. As a Firefox engineer, I want thresholds from BugBot, so that worklens agrees with the nags I already get.
7. As a Firefox engineer, I want an item with no timestamp left unflagged, so that missing data is never reported as neglect.
8. As a Firefox engineer, I want new stale items to look like today's ("Waited:"), so that one idea has one look.
9. As a Firefox engineer, I want an item to stop being stale as soon as it has activity, so that the flag is current.
10. As a Firefox engineer, I want stale items to keep their section, so that staleness changes emphasis, not placement.
11. As an agent using `--json` or MCP, I want the stale flag and the quiet days, so that I can tell the user what's gone quiet.
12. As a Firefox engineer, I want the existing seven-day stale cases unchanged, so that nothing I rely on shifts.

## Implementation Decisions

- Extend the existing per-placement `stale` flag. Don't build a second mechanism.
- One named threshold per new case in the rules module, values from BugBot's inactivity rules. This spec doesn't fix the numbers until they're checked.
- Activity time per case: revision modified time, Jira `updated`, bug `last_change_time`. A missing time means not stale.
- Reason text for stale cases gains the quiet days, in `ReasonText`.
- JSON keeps its existing stale fields; any days field is additive under `worklens/2`. Update the skill if a field is added.
- `busy.json` gains a stale case for each new rule.

## Testing Decisions

- A good test pins `now` and the fixture timestamps and checks the stale flag and reason. Expected days come from the fixture.
- `build_snapshot` seam: one day either side of each threshold for each case, and an item with no timestamp that stays unflagged.
- `core/tests/busy.rs` keeps passing.
- Prior art: `core/tests/rules.rs`, `core/tests/review_age.rs`.
- Seen red: change a threshold or the comparison in the source; the boundary tests must fail and mutants must die.

## Out of Scope

- A threshold setting in config or UI.
- Teammates' stale work.
- Nagging or commenting; worklens stays read-only.

## Further Notes

- Check before building: BugBot's inactivity thresholds, from its rule configs.
