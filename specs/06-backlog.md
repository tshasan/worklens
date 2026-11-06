# Backlog: remove the separate TUI destination

Status: scope reduction required; CLI/MCP compatibility retained.
Source: SPEC.md. This replaces the previous Backlog feature spec.

## Decision

Assigned, unstarted work is useful for reference and planning. It does not
justify another permanent TUI destination alongside Sprint and Everything
else. Remove Backlog from TUI navigation, without losing access to the items
or breaking the branch's existing machine interfaces.

## Required behavior

- No Backlog tab, section-menu destination, navigation stop or section key
  in the TUI, including after loading saved preferences.
- Items whose primary placement is Backlog appear in Everything else in the
  TUI, retaining their identity, factual reason and open-item action.
- Higher-priority placements still win. Sprint tickets remain accessible
  in Sprint; needinfo and review requests remain in their action sections.
- Keep `worklens backlog`, section IDs and existing JSON/MCP fields intact.
  Prefer retaining core classification and projecting it into the existing
  TUI list over changing shared snapshot semantics.
- Do not reorder persisted section indexes or reinterpret a saved preference
  for another section. Inspect the preference format before implementation.
- TUI counts, filtering, selection restoration, hidden/done items and refresh
  use the same effective list. An item is not duplicated within that list.
- No new query syntax, mode, filter control or setting.
- Age wording names the timestamp actually available. Creation time must
  never be labeled assignment time. Missing revisions are not conclusive
  proof that no work has begun; describe source state conservatively.
- Update help and the agent skill to distinguish CLI availability from TUI
  placement. Keep useful core fixtures and coverage.

## Acceptance

Exercise real TUI update/view code with busy and Bugzilla-only fixtures.
Unstarted assigned bugs and tickets can be selected and opened in Everything
else; Backlog cannot be navigated to. Filter, refresh, done/restore and saved
preferences do not strand those items or change unrelated preferences.
CLI/MCP output retains the Backlog interface. Seen-red tests must catch lost
items, inconsistent counts and a navigation stop for the removed tab.
