# Honour rate limits

Status: implemented; verification required. Source: SPEC.md.

## Revised scope

Verify the implementation already on this branch; do not rebuild it. The
original text below describes the intended behavior, not current absence.
Focus on retry timing, per-service cooldowns, bounded waits and attempts,
cache retention and source status. Keep deterministic timing tests. Existing
call-log fields may remain, but adding metrics for spec 11 is not required.
Spec 11 is deferred. Follow SPEC.md's validation and evidence requirements.

## Original design reference

The following describes the original proposal. It is subordinate to the
revised scope and is not an instruction to expand the implementation.

## Problem Statement

When Bugzilla, Jira or Phabricator tells worklens to slow down, worklens
waits less than it was asked to. A `Retry-After` of two minutes is cut to ten
seconds, and the HTTP-date form of `Retry-After` is read as one second. Each
fetch also decides on its own, so a refresh that fans out dozens of requests
to one service hits the limit again from every request at once. The result is
a refresh that fails with rate-limit errors, a source that shows as broken in
`worklens health`, and lists I can't trust because they came from a partial
fetch.

## Solution

Worklens waits as long as the server asks, in either form `Retry-After`
takes. Once one request to a service is told to wait, every request to that
service waits for the same cooldown instead of each finding out separately.
Retries without a server hint back off exponentially with jitter. When the
asked delay is longer than worklens is willing to wait inside one refresh,
the source is marked failed with a reason that says when to try again,
instead of retrying early.

## User Stories

1. As a worklens user, I want worklens to wait the number of seconds a server asks for in `Retry-After`, so that the retry succeeds instead of being limited again.
2. As a worklens user, I want an HTTP-date `Retry-After` read as a date, so that a server asking me to wait until 07:28 isn't retried after one second.
3. As a worklens user, I want a `Retry-After` in the past or of zero treated as "retry now", so that a stale header doesn't stall refresh.
4. As a worklens user, I want an unparseable `Retry-After` to fall back to the normal backoff, so that a malformed header can't crash or hang a refresh.
5. As a worklens user, I want all requests to one service to share one cooldown, so that a fan-out of 40 Bugzilla requests doesn't trip the limit 40 times.
6. As a worklens user, I want a cooldown on Bugzilla not to delay Jira or Phabricator, so that one busy service doesn't slow the whole refresh.
7. As a worklens user, I want retries without a hint to back off exponentially with jitter, so that worklens doesn't retry in lockstep with itself.
8. As a worklens user, I want a delay longer than a refresh can afford to fail the source with "rate limited, try again after 14:32", so that I know when the data will be fresh and don't retry blindly.
9. As a worklens user, I want a rate-limited source to keep showing its cached data marked stale, so that a limit doesn't empty my lists.
10. As a worklens user, I want `worklens health` to say a source was rate limited and for how long, so that I can tell a limit from an outage or a bad token.
11. As a worklens user in the TUI, I want the source status line to say "rate limited" rather than a generic failure, so that I don't go re-entering my token.
12. As an agent using `--json` or MCP, I want a rate-limited source reported in the existing per-source error shape, so that I can tell the user to wait instead of reconfiguring.
13. As a worklens user, I want non-retryable statuses (401, 403, 404) still to fail at once, so that a bad token isn't retried.
14. As a worklens user, I want the number of attempts per request still bounded, so that a refresh always finishes.
15. As a maintainer, I want the retry decision to stay a pure function of status, header, attempt and the current time, so that it is testable without a clock or network.

## Implementation Decisions

- The retry decision in the HTTP client keeps its shape: given status, the raw `Retry-After` value, the attempt number and `now`, it returns an optional delay. It gains `now` so it can turn an HTTP-date into a duration.
- Both `Retry-After` forms are parsed: delta-seconds and HTTP-date (RFC 9110). A date in the past means zero.
- The ten-second cap is removed as a cap on how long to wait. It is replaced by a refresh budget: a delay within the budget is waited; a delay beyond it ends the request with a rate-limit error that carries the retry-at time.
- A cooldown is held per service host on the shared client. Before sending, a request waits out any active cooldown for its host. A 429 or 503 with `Retry-After` sets the cooldown for the host.
- Backoff without a hint is exponential from one second with full jitter. Jitter takes its randomness from an injected source so tests are deterministic.
- The rate-limit failure is a distinct error kind in the source error, so `health`, the TUI status line and JSON can name it. Existing error kinds and JSON field names are unchanged; any new field is additive under `worklens/2`.
- The call log records the wait time per call so spec 11 (refresh cost) can report it.
- No change to what is fetched or to core.

## Testing Decisions

- Good tests here check observable behaviour: how long the client waits and what error it reports, given a recorded server response. They don't inspect cooldown internals.
- Unit tests on the pure retry decision, in the existing style of the HTTP module's tests: delta-seconds, HTTP-date in future and past, zero, garbage, cap removed, attempts exhausted, non-retryable statuses. Expected delays come from the header values in the test, not from calling the function.
- Replay seam: a `WORKLENS_REPLAY` route set where Bugzilla answers 429 with `Retry-After` once, then 200. The binary must make exactly the recorded calls. A second route set returns a delay beyond the budget; `worklens health --json` must report the source as rate limited with the retry-at time from the fixture header.
- Shared cooldown: a replay where the first of several Bugzilla requests is limited; the call log must show no other Bugzilla request sent before the cooldown ends, and Jira requests unaffected.
- Prior art: the existing retry tests in the HTTP module, the replay setup in the binary's tests, and `tests/robust.rs` in the worklens crate (add a mutated `Retry-After` header to the damaged-input cases).
- Tests must sleep on an injected clock, never real time, so `cargo test` stays fast.

## Out of Scope

- Reporting request counts, bytes and time per refresh (spec 11).
- Reducing the number of requests a refresh makes.
- Any change to which endpoints are called.

## Further Notes

- Bugzilla, Jira Cloud and Phabricator each send `Retry-After` differently; record one real limited response from each before building if possible, and keep them as fixtures.
