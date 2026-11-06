# Backout alert

Status: implemented; attribution audit required. Source: SPEC.md.

## Revised scope

The code already exists. Audit and fix its evidence, rather than adding a
new backout feature. Bug-number matching in the original text below is only
candidate discovery: it cannot establish which author's patch was reverted.
Correlate affected changesets/revisions where supported. Ambiguous matches
must not produce a definite personal-backout claim. Another patch on the same
bug must not clear an unresolved backout. Check repository propagation and
repeat backouts. Failure links must concern the backout evidence, not an
unrelated earlier push. Add focused same-bug/multiple-author and repository
regressions. SPEC.md overrides the original matching and clearing rules.

## Original design reference

The following describes the original proposal. It is subordinate to the
revised scope and is not an instruction to expand the implementation.

## Problem Statement

When a sheriff backs out my patch, worklens doesn't tell me. The landing
scan sees the backout commit and throws it away so it doesn't count as a
landing. Phabricator keeps the revision closed, so the bug sits in
"Everything else" with no reason, and the evidence line admits "later
backouts and shipped builds unverified". If the sheriff also needinfos me,
the needinfo rule fires, but nothing says the patch was backed out or why.
A backout is the most urgent thing that can happen to landed work: my patch
isn't shipping, the tree broke, and it's my turn. Today I learn about it
from bugmail.

## Solution

Worklens keeps backout commits from the landing scan and matches each one to
my landing by bug number. A backed-out landing goes to the top of Waiting on
you with a reason that states the facts: where it was backed out, how long
after landing, and the sheriff's stated cause, verbatim. The next step opens
the backout push, and its failure log when push health has one. When I
re-land the bug, the alert clears.

## User Stories

1. As a Firefox engineer, I want to see in Waiting on you that my patch was backed out, so that I learn it before reading bugmail.
2. As a Firefox engineer, I want the reason to say which tree it was backed out of, so that I know whether it's autoland or central.
3. As a Firefox engineer, I want the reason to say how long after landing the backout came, so that I can judge whether it was caught by the first push or later.
4. As a Firefox engineer, I want the cause copied verbatim from the backout subject, so that worklens never paraphrases a sheriff into something misleading.
5. As a Firefox engineer, I want "Backed out of autoland 3h after landing: causing bc failures" as the reason, so that I can act without opening anything.
6. As a Firefox engineer, I want the next step to open the backout push, so that I see the failing jobs in Treeherder.
7. As a Firefox engineer, I want the next step to open the failure log directly when push health has one, so that I skip a click.
8. As a Firefox engineer, I want the alert to clear when I re-land the bug, so that a fixed backout doesn't linger.
9. As a Firefox engineer, I want a re-land that is itself backed out to alert again, so that a second backout isn't hidden by the first re-land.
10. As a Firefox engineer, I want a backout of someone else's patch on my bug not to blame me, so that I'm not sent to fix work that isn't mine.
11. As a Firefox engineer, I want a "revert" or "Backout" subject recognised as well as "Backed out", so that wording differences don't hide a backout.
12. As a Firefox engineer, I want a backout subject with no "for …" clause to show "cause not stated", so that a missing cause isn't invented.
13. As a Firefox engineer, I want a backout that names several bugs to match each of them, so that a multi-bug backout alerts every affected bug.
14. As a Firefox engineer, I want the backout to outrank the needinfo the sheriff left, so that the item says the more specific fact.
15. As a Firefox engineer, I want the evidence line to stop saying "later backouts unverified" once the scan checked for them, so that the evidence is honest.
16. As a Firefox engineer, I want a backed-out item kept even though its revision is closed, so that "landed" filtering doesn't drop it.
17. As a Firefox engineer, I want the help legend to explain the `bo` tag, so that I know what it means.
18. As an agent using `--json`, `--compact` or MCP, I want the backout reason and its link in the entry, so that I can tell the user their patch was backed out.
19. As an agent, I want `worklens next` to return a backed-out item first, so that the most urgent action is surfaced.
20. As a Firefox engineer, I want the TUI to show the backout reason without layout breaking on long sheriff subjects, so that the list stays readable.

## Implementation Decisions

- Sources: the landing scan stops discarding backout commits. It returns them as a second list alongside landings: each with tree, push time, push link, the bug numbers named in the subject, and the cause text. The scan remains a thin fetch plus a pure parse.
- Matching is by bug number in the subject ("Bug 1995008"). A backout matches my landing on that bug if it comes after it on the same tree, or after its merge to a later tree.
- The cause is the subject's text after the first " for ", trimmed, verbatim. No " for " means no cause.
- Raw gains the backout list with `#[serde(default)]`. Bump `RAW_VERSION` only if old caches would mislead.
- Core: new `Reason::BackedOut` with its own `RULES` row in Waiting on you, `Facets` tag `bo`, kept when landed, sorted first within the section. Its `next_step` opens the backout push, or the push-health failure log when the raw data has one. Its text lives in `ReasonText`.
- Placement: a backout after my latest landing of the bug fires the rule. A landing after the latest backout clears it. A backout whose matched landing isn't mine doesn't fire.
- Rule order: `backedOut` ranks above needinfo, so the sheriff's needinfo becomes secondary on the same item.
- Evidence: when the scan ran, the "later backouts unverified" clause is replaced by what it found.
- JSON: the reason appears through the existing reason fields; any new field (backout link, cause) is additive under `worklens/2`. Update the skill.
- `busy.json` gains a backed-out landing so the busy test keeps firing every rule.

## Testing Decisions

- A good test feeds commit subjects and push data and checks the reason and next step a user sees. It does not inspect the scan's intermediate structs.
- Pure-parse fixture tests in sources for the landing scan: a land, a backout, a re-land, a backout of someone else's patch on my bug, a "revert" subject, a subject with no cause, and a multi-bug backout. Expected bug numbers and causes come from the fixture subjects.
- `build_snapshot` seam: the busy fixture and a focused fixture. Assert the reason text against the fixture's subject and timestamps ("3h" computed from the fixture times, by hand, in the test). Assert the re-land clears it and the second backout re-fires.
- Replay seam: `worklens waiting --json` against a recorded push log shows the backout entry with its link.
- The oracle in testkit gains a screen with a long backout cause, to be caught if it overflows; add to both lists.
- Prior art: the existing landing-scan tests in sources, `core/tests/rules.rs`, `core/tests/evidence.rs`, `core/tests/busy.rs`.
- Seen red: break the subject matcher and the re-land clearing in the source and watch the tests fail; `cargo mutants --in-diff` leaves nothing surviving in the matcher.

## Out of Scope

- Fetching failure classifications beyond what push health already returns.
- Detecting backouts on branches worklens doesn't already scan.
- Any write to Bugzilla (no auto-needinfo, no comments).

## Further Notes

- Unknown to check first: that Phabricator does keep the revision closed after a backout. Look up a recent backout of my own before building.
