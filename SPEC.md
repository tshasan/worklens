# Worklens: finish the attention tool

Revised November 5, 2026. This replaces the previous feature roadmap and its
build order. Read this file before any numbered spec. If an older passage in
a numbered spec conflicts with this file or its revised-scope section, the
revised scope wins.

## Thesis

Worklens tells one Firefox engineer where their work needs attention, why,
and where to act, by reconciling the tools that each hold part of the truth.
Its organizing question is: whose turn is it?

The problem is reconstructing obligations across Bugzilla, Phabricator,
Lando, Treeherder and Jira. The product earns its place by connecting facts
that would otherwise be missed. A normal session should let the user find
an obligation, understand its evidence, and open the authoritative tool.

## Ethos

- Earn attention: prominent items explain why they need the user.
- State evidence precisely. Inactivity is observable; blockage and urgency
  require further evidence. Assignment is not proof of who resolved a bug.
- Keep uncertainty visible. Missing or stale data never means no work.
- Show reason, next step and supporting facts, with a useful target link.
- Keep ordinary use quiet: show the cache immediately, preserve selection,
  and add no unnecessary tabs, settings, modes or shortcuts.
- Stay read-only toward upstream services. CLI, TUI and agents share core
  calculations and facts.

A feature needs a recurring decision it makes easier. Available API data,
implementation ease and an existing spec are not sufficient justification.

## Current state and assignment

The starting branch is `feat/spec-features`. Specs 01 through 07 have
implementations committed; HEAD `c4c28c0` partially implements spec 08.
The working tree was clean when this handoff was written. This is a starting
inventory, not evidence that those implementations are correct or complete.
Read the current diff against `main` before editing; do not rebuild completed
features from the original problem statements.

Complete the bounded work below. Do not continue the old feature sequence.
No new product features are authorized by this handoff. Follow AGENTS.md and
AESTHETICS.md. Do not revert unrelated changes or rewrite branch history.

| Spec | Decision | Remaining work |
| --- | --- | --- |
| 01 Rate limits | Keep; verify | Cooldown, retry timing, bounded refresh and truthful source status |
| 02 Backouts | Keep; verify and fix | Attribution, repository boundaries, re-landings and evidence links |
| 03 Landed | Keep as reference | Accurate release claims, source uncertainty and quiet presentation |
| 04 Lands-in | Keep; verify | Shared calculation, date boundaries, missing/stale calendar and interface agreement |
| 05 Stalled | Keep narrowly | Accurate inactivity wording, justified thresholds and no invented urgency |
| 06 Backlog | Remove separate TUI destination | Keep CLI/MCP compatibility; retain items in existing TUI lists |
| 07 Week | Withdraw branch feature | Remove branch-only reporting and its extra fetch; no history project |
| 08 Reviewer away | Finish conditionally | Verify real response, complete tests and output; withdraw if unsupported |
| 09 Parent rollup | Deferred | No grouping toggle, nested hierarchy or parent-count fetching |
| 10 Triage duty | Deferred | No section, discovery or fetches |
| 11 Refresh cost | Deferred | No new reporting or instrumentation project |

## Work order

### 1. Establish trustworthy evidence

Audit the implemented backout and release features first. Check whether
fixtures represent the actual source response and whether output claims
more than the data proves. Fix demonstrated gaps with focused regression
coverage; do not expand supported services or scanned repositories.

Backout bug numbers are candidates, not proof that a particular patch was
reverted. Attribute to the affected changeset/revision when evidence permits.
Handle two authors or multiple patches on one bug, propagation between trees,
re-landings and repeat backouts. Another patch landing on the same bug must
not clear an unresolved backout. Ambiguous evidence must not accuse the user
or claim their patch is definitely gone. Link to the relevant backout/failure
evidence, not unrelated earlier landing failures.

Release flags and calendars establish different facts. Distinguish a fixed
status, expected train position, and a build confirmed to contain a fix.
Do not call a fix shipped solely because its version is below Beta. Verify
calendar freshness, release/merge boundaries, missing data and the current
release number where available. If build inclusion is unverified, say so.
Never infer a fixed version from a date. Keep the calculation in core and
consistent across TUI, JSON, compact output and MCP.

Verify the rate-limit implementation already on the branch. Honour server
retry times, isolate service cooldowns, bound attempts and total waiting,
and retain cached data with clear rate-limit status. Test timing without
long real sleeps. Do not add refresh-cost reporting to finish this work.

### 2. Close reviewer-away or withdraw it

Use spec 08's revised scope. First verify a real `user.search` availability
response through the existing read-only source access, if configured. Keep
only a sanitized fixture; never record credentials or personal data beyond
what is needed. Public authoritative source evidence may establish the shape
if a recorded response is unavailable, but report token-specific access as
unverified. Do not present synthetic fixtures as recorded responses.

If the response/access cannot support a reliable feature, remove the partial
spec 08 implementation and report why. This fallback completes this part of
the assignment; do not leave half-finished code or add calendar integrations.

### 3. Make inactivity factual

Use spec 05's revised scope. Audit the actual timestamps behind each reason.
Review-request age, revision modification, bug modification and Jira updates
are different facts. Name the measured event. Verify threshold provenance;
do not copy an unrelated BugBot threshold onto a new case. An unsupported
new stale case should be withdrawn rather than assigned an invented policy.
Keep items in their existing sections. Age alone must not assert a blocker,
rebase requirement, neglect or immediate obligation.

### 4. Reduce scope already added

Spec 06 removes Backlog from TUI navigation while preserving its existing
CLI/MCP/JSON use and access to those items through the TUI. Do not add a new
filter language, mode or setting to compensate.

Spec 07 withdraws the branch-only weekly summary, MCP tool, documentation
and extra review-history fetching. Do not introduce event-history fetching.
Preserve established contracts from `main`; audit the diff before deleting
shared code, fixtures, fields or fetches. Old cached additive fields should
remain readable. Update the agent skill to match the final surfaces.

Landed remains a reference destination, after actionable sections. It must
not add warning color or urgent next-step language for ordinary completed
work. Respect existing visibility preferences; no new preference is needed.

### 5. Validate and report

Run the checks required by AGENTS.md:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use the explore workflow in `.claude/skills/explore` to exercise the real TUI
headlessly. At 80x24, with the busy and Bugzilla-only fixtures, find the first
obligation, read why, open actions with Enter and close with Esc. Check
filter match/no-match/clear, done/restore, resize, refresh while reading,
failed sources, no cache and an empty queue. Check dark, light and NO_COLOR.
Verify Backlog items remain reachable and Landed does not crowd out actions.
A strip fitting the terminal is necessary, not sufficient for usable
navigation.

For changed behavior, meet AGENTS.md's seen-red and mutation requirements,
keep saved regressions, and run the required frame-budget check with 1500
items. Do not delete or weaken oracle cases to make a removed tab pass;
replace obsolete tab expectations with assertions that preserve item access
and still catch the original class of problem. If required tooling or source
access is unavailable, report the exact unverified requirement.

The final handoff must state what was fixed, retained and withdrawn; checks
actually run and their results; remaining evidence limitations; and any
compatibility consequences. Do not describe an unavailable check as passed.

## Explicitly deferred

Parent grouping, triage-duty discovery, refresh-cost reporting, historical
activity reporting, team/org dashboards, new sources, new upstream writes,
and new modes or configuration are outside this assignment. The deferred
specs record ideas, not instructions to implement them. Reconsider only when
real daily use identifies a recurring decision the current tool cannot help
with.
