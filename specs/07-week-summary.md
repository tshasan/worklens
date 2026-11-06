# Week summary: withdraw from this branch

Status: deferred; branch implementation must be removed.
Source: SPEC.md. This replaces the previous week-summary feature spec.

## Decision

Weekly reporting is not needed to finish the attention tool. The current
implementation cannot equate a Done ticket's recent `updated` timestamp
with a completion event, or current assignment with who resolved a bug.
Do not build event-history fetching to rescue the feature in this release.

## Required work

- Remove the branch-added `worklens week` command and `worklens_week` MCP
  tool, including help, agent instructions and advertised capabilities.
- Remove branch-only summary construction, rendering and serialization.
- Remove the extra reviewed-history fetch and parsing used only by this
  feature. Ordinary review fetching must remain intact. A failure in a
  withdrawn optional fetch must no longer affect source health.
- Remove obsolete week-only tests and fixtures. Preserve shared tests and
  data needed by retained features, especially Landed and normal reviews.
- Audit against `main` before removing any public field or surface. Preserve
  established `worklens/2` contracts. This spec authorizes withdrawal of the
  branch-only week feature, not unrelated schema changes.
- Caches produced before withdrawal must still deserialize; harmless
  additive fields can remain ignored or retained for compatibility.

## Acceptance

CLI help, MCP tool discovery and the agent skill no longer advertise week
reporting. A replay refresh no longer requests week-only review history;
ordinary reviews, landings and failed-source behavior continue working.
An existing cache containing the branch's reviewed-history data loads.
The required workspace checks pass.

## Reconsideration

Reopen only after recurring reporting needs justify it and source evidence
can establish event time, actor and coverage. Reporting must not depend on
which attention section currently owns an item. No weekly TUI view is planned.
