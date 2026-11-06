# Reviewer away

Status: partially implemented; verify source before completion. Source: SPEC.md.

## Revised scope

First verify the availability response and access as described in SPEC.md.
If reliable support cannot be established, withdraw the partial spec 08 code
and document the evidence limitation. Do not leave it half-finished.
If supported, finish pure-parse fixtures, build_snapshot cases, busy fixture,
replay output, additive reviewerAway JSON and the agent skill. Retain the
existing review section; add no tab, setting or calendar source.
A known away state and return date are facts, not proof of why a review is
stuck. Suggest considering another reviewer; do not imply adding one is
required. Audit blocking versus optional reviewer semantics. Mixed reviewer
availability and group reviewers must not produce a false claim that nobody
can review. Expired availability must not remain an active away warning;
unknown or failed lookups must not be described as confirmed availability.
Test available, away, one-of-two away, blocking/optional combinations, groups,
expired dates and failed lookup without hiding ordinary reviews. SPEC.md
and this section override conflicting wording or unconditional actions below.

## Original design reference

The following describes the original proposal. It is subordinate to the
revised scope and is not an instruction to expand the implementation.

## Problem Statement

"Waiting on others" shows who has my review and for how long. Nothing reads
Phabricator availability, so a reviewer on PTO for two weeks looks the same
as one who'll review tomorrow. The fix, adding another reviewer, is cheap,
but only if I know.

## Solution

Worklens reads reviewer availability from Phabricator. When a reviewer on my
in-review revision is away, the reason says so with their return date:
"D123 waiting on quillfox, away until Nov 18". The next step is "Add another
reviewer", linking to the revision.

## User Stories

1. As a Firefox engineer, I want to see that my reviewer is away, so that I add another reviewer instead of waiting.
2. As a Firefox engineer, I want the return date in the reason, so that I can decide whether waiting is fine.
3. As a Firefox engineer, I want "D123 waiting on quillfox, away until Nov 18", so that the reason states a checkable fact.
4. As a Firefox engineer, I want the next step "Add another reviewer" with the revision link, so that I can act in one step.
5. As a Firefox engineer, I want an available reviewer to show no availability note, so that the list isn't cluttered.
6. As a Firefox engineer, I want a revision with several reviewers flagged only when every blocking reviewer is away, so that I'm not told to add reviewers when one is around.
7. As a Firefox engineer, I want a group reviewer never flagged as away, so that projects aren't treated as people.
8. As a Firefox engineer, I want an away reviewer whose return date has passed treated as available, so that stale availability doesn't mislead.
9. As a Firefox engineer, I want availability lookups batched, so that refresh doesn't make one call per reviewer.
10. As a Firefox engineer, I want availability failing to load to leave reasons as they are today, so that a lookup failure doesn't hide my reviews.
11. As an agent using `--json` or MCP, I want the away reviewer and the date on the entry, so that I can suggest adding a reviewer.
12. As a Firefox engineer, I want the TUI to show the away note without breaking layout, so that the list stays readable.

## Implementation Decisions

- Sources: one batched `user.search` call for the reviewer PHIDs on my in-review revisions, reading availability and its `until` date. Thin fetch, pure parse, fixture test. Raw gains the availability map with `#[serde(default)]`.
- Core: availability modifies the in-review reason rather than adding a section. If a new `Reason` variant is cleaner than a field on the existing one, add it with a `RULES` row, `Facets`, `next_step` and `ReasonText` as AGENTS.md says, and a `busy.json` case.
- Away means availability says away and `until` is after `now`.
- Next step "Add another reviewer" reuses the existing find-another-reviewer step if it fits.
- JSON: additive `reviewerAway: { who, until }` on the entry under `worklens/2`. Update the skill.

## Testing Decisions

- A good test records a `user.search` response and checks the reason a user reads; the date comes from the fixture's `until`.
- Pure-parse fixture test in sources: one available reviewer, one away.
- `build_snapshot` seam: away reviewer, available reviewer, two reviewers with one away (not flagged), a passed `until` (not flagged), a group reviewer (not flagged).
- Replay seam: `worklens others --json` with a recorded `user.search`.
- Prior art: the Phabricator parse tests in sources, `core/tests/review_age.rs`, `core/tests/busy.rs`.

## Out of Scope

- Adding reviewers from worklens; it links to Phabricator.
- Calendars or PTO sources other than Phabricator.

## Further Notes

- Check before building: that `user.search` returns availability to my token. Record one call.
