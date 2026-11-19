# Drift compares a bug against the wrong Jira issue

Status: implemented 2026-11-19. Found 2026-11-19 comparing `worklens drift` with Jira.

## Problem

When a bug links to more than one Jira issue, drift can pick a product
"idea" (AIDI) instead of the engineering ticket and report disagreements
that don't exist.

## Evidence

Bug 2091535 drift reason:

> Jira is Backlog but a revision is in review; expected ENG REVIEW; AIDI-334
> is in no sprint, but the bug has an open revision; assignee: Bugzilla
> thasan@mozilla.com, Jira unassigned

The real ticket, AIPLAT-1674, is assigned to me, in ENG REVIEW, and in the
active sprint. Worklens itself lists AIPLAT-1674 in "Sprint". The same bug
also appears in Team drift with the same false reason.

## Expected

- When a bug maps to several Jira issues, status, sprint and assignee checks
  use the engineering ticket (the one in a sprint or assigned to the bug
  assignee), not an idea/epic it rolls up to.
- AIDI ideas count as rollups (as the "Ideas" section already treats them),
  never as the ticket whose status should track the revision.
