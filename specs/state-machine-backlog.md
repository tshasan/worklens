# State machine reliability backlog

Status: deferred at the user's request until they resume this work.
Audit date: November 6, 2026.

Resolve lifecycle failures first, then make the state needed to prevent them
explicit. The first four ranked findings come from static inspection and need
failing reproductions before implementation. The remaining items are
investigations or bounded refactors, not claims that every listed lifecycle is
broken.

This backlog covers the current working tree, including local-commit support.
It complements [SPEC.md](../SPEC.md); deferred product features remain deferred.
Creating this backlog does not begin implementation.

Use small, independent enums where they prevent invalid combinations. Keep
pure classifications in core, take `now` explicitly, and prefer indexes and
plain functions. Preserve existing cache and agent-output compatibility.

## Work order

The rank is the recommended execution order. It balances user impact, likely
exposure, evidence of a gap, implementation effort, and dependencies. It
replaces the initial P1/P2/P3 grouping; item IDs remain stable. Details below
stay in ID order for reference.

Impact describes the consequence of the failure being addressed:

- High: refresh stops working, obligations stay hidden, local state is lost,
  or evidence can lead to the wrong action.
- Medium: stale or misleading presentation, recoverable disruption, or a
  maintenance risk with an indirect user effect.

Effort is a relative estimate for the full acceptance criteria, including
regression coverage and required checks. S is a focused change with an
existing test seam. M requires coordinated changes or several transition
cases. L spans subsystems, compatibility decisions, or difficult failure
testing. These are planning estimates, not measured durations.

Each checkbox closes when its acceptance criteria are verified, or an
investigation establishes that existing behavior already satisfies them. A
reproduced data-loss or incorrect-evidence case moves ahead of lower-impact
work even when its broader investigation ranks later.

| Done | Rank | ID | Impact | Effort | Work and expected effect | Depends on |
| --- | --- | --- | --- | --- | --- | --- |
| [ ] | 1 | SM02 | High | S | Arm refresh from fresh cache; keep normal sessions current | — |
| [ ] | 2 | SM01 | High | M | Reject late progress; prevent a permanently busy refresh | — |
| [ ] | 3 | SM03 | High | M | Wake expired snoozes; bring obligations back on time | — |
| [ ] | 4 | SM04 | Medium | M | Commit settings consistently; avoid applying unsaved values | — |
| [ ] | 5 | SM06 | Medium | M | Acknowledge the activity actually read; keep unread cues useful | — |
| [ ] | 6 | SM05 | Medium | M | Retain touched items; reconcile upstream edits after failures | SM01 |
| [ ] | 7 | SM14 | High | S | Detect pagination cycles; bound runaway refresh requests | — |
| [ ] | 8 | SM09 | High | L | Preserve unsaved marks; protect done, snooze, and read state | — |
| [ ] | 9 | SM11 | High | L | Align health and evidence; expose stale or incomplete facts | — |
| [ ] | 10 | SM12 | Medium | L | Restore the terminal and clean up owned child processes | — |
| [ ] | 11 | SM10 | Medium | L | Advance time-based facts; keep long-running sessions accurate | SM02, SM03 |
| [ ] | 12 | SM13 | High | M | Verify retries and cooldowns; recover predictably from limits | SM11 |
| [ ] | 13 | SM15 | Medium | M | Verify cache transitions; avoid stale supplemental comparisons | SM11 |
| [ ] | 14 | SM08 | Medium | M | Match feedback to operations; preserve meaningful error reports | SM01, SM04 |
| [ ] | 15 | SM16 | High | L | Verify write and credential failures; preserve durable state | SM04, SM09 |
| [ ] | 16 | SM07 | Medium | L | Bind dialogs to targets; preserve selection during changes | — |
| [ ] | 17 | SM17 | Medium | L | Verify parser boundaries and replay; contain malformed inputs | — |
| [ ] | 18 | SM18 | Medium | L | Consolidate conflicting rules; reduce future interpretation drift | SM10, SM11 |

Start with SM02, SM01, and SM03: they address everyday freshness and visibility
with bounded changes. SM04 through SM06 repair concrete consistency gaps.
SM14 is a small guard against a severe but conditional failure. The broader
persistence, evidence, and lifecycle investigations follow. SM13 builds on
existing retry coverage; SM07 already has selection and modal regression
coverage; SM18 proceeds only where a disagreement is demonstrated. Those
items offer less certain incremental benefit than the earlier fixes.

## Item details

### SM01 Prevent late progress from reviving a completed refresh

`Fetched` clears `App.busy`, while `Progress` sets it unconditionally. The
runner receives completion and buffered progress through separate branches,
so progress can arrive after completion and block subsequent refreshes.

Code: [app.rs](../crates/tui/src/app.rs), `begin_refresh` and `update`;
[ui.rs](../crates/worklens/src/ui.rs), `event_loop`.

- Reproduce completion followed by queued progress through the real runner
  and reducer. A later refresh must still start.
- Represent idle, full refresh, and item refetch explicitly. Correlate
  progress and completion with the operation that produced them, or enforce
  equivalent ordering that makes stale messages harmless.
- Cover success, failure, duplicate refresh requests, and messages from an
  older operation arriving during a newer one. At most one fetch runs.

### SM02 Arm automatic refresh when starting from fresh cache

`App::start` skips fetching fresh data but leaves `last_fetch_started` empty.
`on_tick` requires that field before scheduling another refresh.

Code: [app.rs](../crates/tui/src/app.rs), `start`, `begin_refresh`, `on_tick`.

- Starting from fresh cache schedules refresh when that cache becomes due;
  missing or stale cache starts a fetch immediately.
- Distinguish disabled scheduling from waiting until a deadline. Specify how
  completion, failure, interval changes, and a long-running fetch affect the
  next deadline, without producing a retry loop on every tick.
- Test boundary times with supplied timestamps. Cached mode must not trigger
  automatic network access; preserve the existing explicit-refresh policy.

### SM03 Wake timed snoozes while the app is idle

Snooze expiry is evaluated by `marks::apply`, but ticks do not reliably call
it when a deadline passes. An expired item can remain hidden until another
action reapplies marks.

Code: [marks.rs](../crates/core/src/marks.rs), `state_and_changes`;
[app.rs](../crates/tui/src/app.rs), `on_tick` and `marks_changed`.

- A timed snooze becomes visible on the first tick at or after its deadline,
  without a fetch, keypress, selection change, or unrelated mark update.
- Untimed snoozes and done marks retain their current activity-based rules.
  Expiry preserves selection where possible and updates counts and rendering.
- Cover several simultaneous expiries and an empty visible list. Avoid
  reapplying every mark on every tick when no deadline is due.

### SM04 Keep settings consistent when persistence fails

Preferences and some core settings change before `SaveSetting` succeeds.
Source settings change only on success, and a queued refresh still runs after
a save error. Displayed, effective, and persisted values can disagree.

Code: [app.rs](../crates/tui/src/app.rs), `change_setting`, `on_edit_key`;
[ui.rs](../crates/worklens/src/ui.rs), `SaveSetting` handling, `apply_setting`.

- Treat editing as a draft. On save success, commit the same value to the
  displayed preferences and effective configuration before dependent work.
  On failure, retain or restore the previous committed value and report it.
- Start dependent refreshes only after a successful save. Cover a save while
  another refresh is already running and preserve the promised next-refresh
  behavior.
- Exercise a real filesystem save failure and recovery for a view setting,
  a core setting, and a source setting. No new settings UI is required.

### SM05 Retain touched items until reconciliation succeeds

`Focus(true)` drains `touched` before the refetch outcome is known. Focus
received while busy also has no durable pending reconciliation request.

Code: [app.rs](../crates/tui/src/app.rs), `touch` and `update`;
[refetch.rs](../crates/sources/src/refetch.rs), `refetch_items`.

- Distinguish pending items from items currently being checked; retire them
  only after successful reconciliation. Preserve newly touched items.
- A failed refetch leaves items eligible for a later bounded retry. Focus
  regained during another fetch is handled after that operation settles.
- Verify that a full refresh with a failed source does not incorrectly clear
  reconciliation work for that source, and repeated focus events deduplicate.

### SM06 Make read acknowledgment track the activity being read

`resting` uses `Timestamp::MAX` for an acknowledged selection. Its identity is
only the item key. `mark_seen` updates a record only when its signature changes,
although comments can count as new activity without changing that signature.

Code: [app.rs](../crates/tui/src/app.rs), `on_tick`;
[marks.rs](../crates/core/src/marks.rs), `changed_since` and `mark_seen`.

- Replace the timestamp sentinel with explicit idle, dwelling, and
  acknowledged states that identify the activity being acknowledged.
- Verify the 800 ms boundary, leaving and returning to an item, new activity
  on the same selected key, and a new comment with an unchanged signature.
- Acknowledging activity clears its unread state and persists it without
  repeated writes. Preserve the current-session change explanation and
  first-visit initialization behavior.

### SM07 Bind modal actions and selection to stable item identities

Mode-specific data lives outside `Mode`; selection additionally spans row
indexes, remembered keys, and per-section history.

Code: [app.rs](../crates/tui/src/app.rs), `Mode`, `set_raw`, `rebuild_rows`,
`on_key`, `snooze`, and the settings handlers.

- Keep modal payloads with their mode, or centralize entry and exit so a
  dialog cannot silently act on a replacement item or the wrong setting.
- Cover item removal, section movement, filtering, hidden items, and refresh
  while menus or snooze prompts are open. Preserve the existing neighbor
  fallback and cancellation messages.
- Preserve filter Enter/Escape behavior, per-section selection, visibility
  fallback, scroll bounds, and the once-per-session summary dismissal.

### SM08 Associate checks and action feedback with their operation

Checks combine `pending` and `ok`; generic `Done` messages overwrite shared
feedback. A successful unrelated action can clear `action_error`.

Code: [app.rs](../crates/tui/src/app.rs), `Check`, `Msg::Checks`, `Msg::Done`;
[doctor.rs](../crates/worklens/src/doctor.rs);
[ui.rs](../crates/worklens/src/ui.rs), `RunChecks` and `run_effect`.

- Represent pending and completed check outcomes explicitly, preserving the
  distinction between required failures and optional unconfigured services.
- Reopening checks cannot let an older result replace a newer result.
  Leaving the sources page must not invalidate the runner's bookkeeping.
- Define feedback precedence for overlapping actions. Toast expiry must not
  erase an unresolved persistence failure, and an unrelated success must not
  imply that failure was repaired. Use existing presentation surfaces.

### SM09 Preserve unsaved marks through failures and shutdown

The runner maintains a saved baseline separately from live marks and merges
changes into disk state. A failed save must remain distinguishable from a
successful save, including when Quit follows the save effect.

Code: [ui.rs](../crates/worklens/src/ui.rs), `run_effect` and shutdown;
[store/lib.rs](../crates/store/src/lib.rs), `save_marks`;
[marks.rs](../crates/core/src/marks.rs), `upgrade`.

- Advance the saved baseline only after a successful write. Preserve failed
  changes for retry and report a final save failure after restoring the
  terminal rather than losing it in an unseen toast.
- Verify concurrent writers changing different items, same-item conflict
  behavior, deletions, monotonic `last_look`, and old-signature upgrades.
- Define recovery for an unreadable or corrupt marks file so it does not
  silently destroy recoverable state. Test against real temporary files.

### SM10 Advance time-dependent snapshot state without network access

`App.now` advances on messages while `Snapshot.now` is fixed at construction.
Review aging, availability, calendar validity, and other derived facts can
remain tied to the old snapshot time.

Code: [app.rs](../crates/tui/src/app.rs), `on_tick`, `rebuild_snapshot`;
[snapshot.rs](../crates/core/src/snapshot.rs), `next_step`;
[train.rs](../crates/core/src/train.rs), `trusted` and `current`.

- Identify the existing time-based decisions and update their derived state
  at relevant boundaries, without adding network calls or new thresholds.
- Keep cached-mode behavior current across snooze, reviewer-return, age,
  merge/release-date, and calendar-freshness boundaries. Preserve selection.
- Ensure rendering becomes dirty when displayed facts change and stays quiet
  otherwise. Validate the 16 ms frame budget with 1500 items.

### SM11 Keep source health and cached evidence consistent

Source state is derived separately from errors, stale markers, retained
payloads, credentials, and request logs. These projections have different
vocabularies and must still agree about what is known.

Code: [raw.rs](../crates/core/src/raw.rs), `Raw::state`;
[carry.rs](../crates/sources/src/carry.rs);
[health.rs](../crates/worklens/src/health.rs);
[evidence.rs](../crates/core/src/evidence.rs);
[release.rs](../crates/core/src/release.rs).

- Pin transitions among off, unused, successful, failed without data, failed
  with retained data, rate-limited, and recovered states across interfaces.
- Preserve the first stale timestamp through repeated failures; successful
  recovery clears the applicable failure and stale status. Check partial
  item refetches separately from whole-source recovery.
- Cached landing matches alongside a repository error must remain visibly
  stale or uncertain in every projection. A failed or disabled source must
  not manufacture empty work or verified evidence. Never carry another
  identity's private work into the current snapshot.

### SM12 Guarantee terminal restoration and child cleanup

Terminal initialization, command handoff, event-stream recreation, and final
restoration are spread through the runner. Child actions have different
timeout and cancellation policies.

Code: [ui.rs](../crates/worklens/src/ui.rs), `enter`, `leave`, `event_loop`,
`open`, `copy`, `run_quietly`, `run_in_terminal`;
[secrets.rs](../crates/worklens/src/secrets.rs), `without_echo`.

- Restore terminal mode, focus reporting, and secret-input echo on every
  relevant error path, including failures after initialization or re-entry.
- Define ownership on timeout and quit: owned quiet/clipboard processes must
  be reaped; browser handoff must retain its intended independent lifetime.
- Exercise command failure, timeout, return to the TUI, and quit with active
  work using controlled local processes and a terminal test seam.

### SM13 Verify HTTP retries and cooldown recovery

Code: [http.rs](../crates/sources/src/http.rs).
Reuse [spec 01](01-rate-limits.md) rather than starting another retry design.

- Verify permit acquisition, cooldown checks before sending, bounded attempts,
  shared wait-budget exhaustion, and restoration of cached retry deadlines.
- Cover concurrent requests, independent services, malformed and expired
  retry hints, and recovery using deterministic replay timing.
- Keep diagnostic state truthful without adding the deferred refresh-cost
  reporting feature.

### SM14 Bound pagination when continuation tokens repeat

Jira and Phabricator loops stop when a token disappears; repeated tokens
currently have no explicit progress guard.

Code: [jira.rs](../crates/sources/src/jira.rs), `pages`;
[phab.rs](../crates/sources/src/phab.rs), `search` and `parents`.

- Detect repeated tokens and cycles, report a source error, and terminate
  without silently reporting an incomplete result as a complete empty list.
- Test normal multi-page results, an empty page with a continuation token,
  a repeated token, a token cycle, and a failed later page through HTTP replay.
- Decide whether a total page/request bound is needed for endlessly changing
  tokens; if added, surface truncation and preserve normal pagination.

### SM15 Verify supplemental and uploaded-diff cache transitions

Code: [supplemental.rs](../crates/sources/src/supplemental.rs);
[local.rs](../crates/sources/src/local.rs), `uploads`;
[fetch.rs](../crates/sources/src/fetch.rs), `cached_diffs`.

- Verify disable/re-enable, first attempt, success, failure retry cadence,
  repository-level fallback, and identity changes across serialization.
- Failed attempts retain valid earlier evidence with its real observation
  time; attempt time must not masquerade as successful observation time.
- Reuse fingerprints only for the matching diff identity. New or missing
  diff evidence yields an unchecked comparison until successfully acquired.

### SM16 Verify durable writes and credential lifecycle failures

Code: [store/lib.rs](../crates/store/src/lib.rs), `write_atomic`, `with_lock`;
[secrets.rs](../crates/worklens/src/secrets.rs);
[setup.rs](../crates/worklens/src/setup.rs);
[ui.rs](../crates/worklens/src/ui.rs), `with_secrets`.

- Verify write, flush, rename, directory-sync, and lock failure semantics.
  Distinguish a failure before replacement from uncertain durability after
  replacement; do not promise rollback after a rename has succeeded.
- Verify keychain/file fallback and removal without losing the usable
  credential. Keep actual credentials out of fixtures and logs.
- Pin setup retry/skip/partial-save behavior and the session lifetime of
  resolved credentials. Document any restart requirement rather than adding
  an unrequested live credential-watching feature.

### SM17 Verify parser and replay termination

Code: [local.rs](../crates/sources/src/local.rs), `fingerprint`;
[tone.rs](../crates/worklens/src/tone.rs), `query`;
[http.rs](../crates/sources/src/http.rs), `Replay::answer`.

- Exercise diff-header and hunk transitions with truncated input, malformed
  counts, multiple files, and metadata-looking content inside hunks. A
  malformed boundary must not silently combine unrelated files.
- Verify partial terminal replies, terminators, timeout, and EOF; preserve
  intended keyboard input where distinguishable from the query response.
- Pin replay route consumption and exhaustion. Unrecorded requests fail
  loudly and never fall through to the network.

### SM18 Consolidate domain state rules where interpretations disagree

Code: [rules.rs](../crates/core/src/rules.rs),
[snapshot.rs](../crates/core/src/snapshot.rs),
[guidance.rs](../crates/core/src/guidance.rs),
[dependencies.rs](../crates/core/src/dependencies.rs),
[evidence.rs](../crates/core/src/evidence.rs),
[release.rs](../crates/core/src/release.rs),
[backout.rs](../crates/core/src/backout.rs),
[local.rs](../crates/core/src/local.rs).

- Compare projections for review/classification, dependency readiness, try
  health and outdatedness, uplift, landing/backout/relanding, release trains,
  telemetry expiry, local synchronization, and cross-system drift.
- Fix demonstrated disagreements through shared pure calculations. Introduce
  typed internal evidence states where strings permit invalid combinations;
  preserve serialized strings and existing agent contracts.
- Keep overlapping sections and independent evidence dimensions explicit.
  Acceptance, landing, and a released build are separate facts. Do not turn
  classifiers into a persisted workflow engine or tighten MCP initialization
  behavior without a separate compatibility need.

## Completion requirements

Each implementation item needs a fixture- or spec-derived expected result and
a regression that runs real worklens code. Demonstrate failure by breaking the
source, not the test. Follow [AGENTS.md](../AGENTS.md) for mutation checks and
saved robust regressions; use HTTP replay only at the network boundary.

Before committing an implementation, run:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For changed TUI transitions, use the explore workflow with busy, Bugzilla-only,
empty, and missing-cache scenarios at 80x24. Keep testkit oracle expectations
intact. For tick or redraw work, run the required frame-budget check; use the
deeper robust hunt when the change warrants it.

Close each item with the reproduction, resulting behavior, checks actually
run, and any remaining limitation. Preserve `worklens/2`, `--compact`, MCP,
and old-cache compatibility; update the agent skill for output additions.
Land implementation changes through focused squash PRs with Conventional
Commit titles, following AGENTS.md.
