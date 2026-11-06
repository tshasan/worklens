# Triage duty

Status: deferred; do not implement in this assignment. Source: SPEC.md.

## Decision

No Triage section, component-owner discovery or untriaged-bug fetching.

Reconsider only after daily use identifies a recurring decision the current
tool cannot help with. The original proposal below is retained for context,
not as an agent task. SPEC.md supersedes its implementation instructions.

## Archived proposal

## Problem Statement

Worklens has no idea which components I triage-own. BugBot nags triage
owners about untriaged bugs by email, and nothing in worklens shows them.
Triage is a standing duty with a weekly cadence; without it beside my own
work it becomes a pile that BugBot escalates.

## Solution

Worklens reads the components where I'm triage owner from Bugzilla and
lists their untriaged bugs, using BugBot's definition: open, priority `--`,
not `[meta]`. A Triage section shows them with a reason like "Untriaged 9
days, Core :: Machine Learning: On Device". The section is hidden when I own
no components. Worklens stays read-only: the next step opens the bug to set
priority and severity.

## User Stories

1. As a triage owner, I want a Triage section of untriaged bugs in my components, so that triage is part of my daily view.
2. As a triage owner, I want "untriaged" to mean what BugBot means, so that worklens and BugBot agree.
3. As a triage owner, I want `[meta]` bugs left out, so that tracking bugs don't count as untriaged.
4. As a triage owner, I want "Untriaged 9 days, Core :: Machine Learning: On Device", so that I see age and component at a glance.
5. As a triage owner, I want the list sorted oldest first, so that bugs close to BugBot escalation come first.
6. As a triage owner, I want the next step to open the bug, so that I set priority and severity in Bugzilla.
7. As an engineer who owns no components, I want the Triage section hidden, so that it doesn't clutter my view.
8. As a triage owner, I want a bug to leave Triage once its priority is set, so that the list shows only remaining work.
9. As a triage owner of several components, I want them all in one list, so that I triage in one pass.
10. As a triage owner, I want the `tr` tag in the help legend, so that I can read the list.
11. As a triage owner, I want an untriaged bug that also needs me (needinfo) to stay in Waiting on you, so that the urgent rule wins.
12. As a triage owner, I want `worklens triage` on the CLI and Triage in MCP sections, so that an agent can help me triage.
13. As a triage owner, I want the count of untriaged bugs on the Triage tab, so that I see the size of the duty.
14. As a triage owner, I want the section strip still to fit 80 columns, so that the TUI works in a standard terminal.

## Implementation Decisions

- Sources: read components where I'm `triage_owner` from Bugzilla's product API, then one query for open bugs in those components with priority `--`, excluding `[meta]`. Thin fetch, pure parse, fixture tests. Raw fields under `#[serde(default)]`.
- Core: `Section::Triage`, `Reason::Untriaged` with a `RULES` row below actionable rules, `Facets` tag `tr`, oldest first. `next_step` opens the bug. Text in `ReasonText`.
- Untriaged age is from bug creation unless BugBot uses another time; follow BugBot.
- Hidden when I own no components: the section is empty and the TUI treats it as `auto` visibility.
- CLI `triage` subcommand; MCP sections include it; JSON additive under `worklens/2`. Update the skill.
- `busy.json` gains two owned components with untriaged bugs and a meta bug that stays out.
- Read-only: no writes to Bugzilla.

## Testing Decisions

- A good test records the product API and bug query and checks the section and reasons.
- Pure-parse fixture tests: owning two components, owning none, a meta bug excluded.
- `build_snapshot` seam: Triage section from `busy.json`; empty on `raw.json` with no owned components; untriaged bug with a needinfo stays in Waiting on you.
- TUI render test: no Triage tab when empty; strip fits 80 columns when present.
- Replay seam: `worklens triage --json` against recorded product and bug responses.
- Prior art: Bugzilla parse tests in sources, `core/tests/busy.rs`, `tui/tests/render.rs`.

## Out of Scope

- Setting priority or severity from worklens.
- Components I'm not triage owner of.
- Team-wide triage views.

## Further Notes

- Check before building: `triage_owner` in the BMO product API, and BugBot's exact untriaged query in its source.
