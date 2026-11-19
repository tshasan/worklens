# In-review work with no Jira ticket isn't flagged

Status: implemented 2026-11-19. Found 2026-11-19 comparing `worklens sprint` with Bugzilla.

## Problem

The sprint view only counts work that has a Jira ticket. Bugs I'm actively
shipping with no ticket are invisible to it, so the sprint shows 16 of 10
points of mostly carried-over work while the current work isn't counted.
Drift doesn't catch this either.

## Evidence

Assigned to me, revisions in review, no Jira ticket of their own:

- Bug 2093725 (4 revisions, try passed)
- Bug 2093723 (2 revisions, try passed)
- Bug 2092358 (3 revisions; only maps to the meta AIPLAT-1607)

Drift reported 2093723 only for a local/Phabricator diff mismatch, and
2093725 not at all.

## Expected

- An assigned bug with an open revision and no Jira ticket (or only a meta
  or epic) is a drift case: "bug has revisions in review but no Jira ticket".
- The sprint view can say how much in-review work is outside the sprint,
  without adding points.
