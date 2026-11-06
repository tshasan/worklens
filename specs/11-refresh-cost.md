# Refresh cost

Status: deferred; do not implement in this assignment. Source: SPEC.md.

## Decision

No new health metrics, endpoint rankings or instrumentation project.
Existing logging may remain; reliability fixes do not depend on this feature.

Reconsider only after daily use identifies a recurring decision the current
tool cannot help with. The original proposal below is retained for context,
not as an agent task. SPEC.md supersedes its implementation instructions.

## Archived proposal

SPEC.md, "Fixes first".

## Problem Statement

I can't tell what a refresh costs. `worklens health` reports requests,
failures and the slowest call per source, but not total time, bytes, or how
much of the time went to rate-limit waits. When refresh feels slow I have no
way to see which source or endpoint is to blame, so I can't tell whether
loading comments and details on selection would help.

## Solution

`worklens health` reports, per source, the requests, total time, bytes and
time spent waiting on rate limits for the last refresh, plus the endpoints
that cost the most. The numbers come from the existing call log, so there
are no new requests.

## User Stories

1. As a worklens user, I want total refresh time per source in `worklens health`, so that I know which source is slow.
2. As a worklens user, I want bytes received per source, so that I can see when a query is pulling too much.
3. As a worklens user, I want request counts per source, as today, so that fan-out is visible.
4. As a worklens user, I want time spent waiting on rate limits shown separately, so that I can tell a slow server from a limited one.
5. As a worklens user, I want the three costliest endpoints per source, so that I know what to optimise.
6. As a worklens user, I want endpoint paths shown without tokens or query secrets, so that pasting health output is safe.
7. As a worklens user, I want `worklens health --json` to include the numbers, so that an agent can diagnose a slow refresh.
8. As an agent using MCP, I want `worklens_health` to include the numbers, so that I can suggest a fix.
9. As a worklens user, I want the numbers from the last refresh only, so that they describe now.
10. As a worklens user, I want `--cached` health to show the last refresh's numbers, so that I can check offline.
11. As a worklens user, I want parallel requests not double-counted in total time, so that the total is wall time.

## Implementation Decisions

- The call log already records per-call status and milliseconds. It gains bytes and wait time (wait time comes from the rate-limit fix, spec 01).
- Health: per-source totals are wall time from first call start to last call end, bytes summed, wait summed, plus the top three endpoints by time with paths stripped of query strings.
- JSON: additive fields on each source in health output under `worklens/2`. Update the skill.
- Plain-text health gains one line per source.

## Testing Decisions

- A good test records a refresh's calls and checks the numbers in health output; expected totals are worked out by hand from the fixture.
- Replay seam: a recorded refresh with known body sizes and two parallel calls; `worklens health --json` totals match the fixture.
- Unit tests on the existing health module for wall-time overlap and path stripping.
- Prior art: the health module's existing tests, `worklens/tests/cli.rs`.

## Out of Scope

- Loading comments and details on selection; that's the follow-up if the numbers show it's needed.
- Historical cost across refreshes.

## Further Notes

- Depends on spec 01 for wait time; without it, report wait as zero.
