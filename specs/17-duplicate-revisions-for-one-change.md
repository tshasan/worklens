# Two open revisions for the same change aren't flagged

Status: implemented 2026-11-19. Found 2026-11-19 listing my open Phabricator revisions.

## Problem

A revision left behind after resubmitting (a new D-number instead of an
update) stays open and waits on reviewers forever. Worklens shows the stack
but doesn't point out the likely stale duplicate.

## Evidence

Bug 2093725 has two open revisions titled "Stop blaming a headless read on an
off-site load that never committed":

- D343316, last modified 2026-11-16
- D343322, last modified 2026-11-18

## Expected

- Two open revisions I authored on the same bug with identical titles show
  as drift: "D343316 and D343322 have the same title; one may be stale".
- Worklens doesn't claim which one is stale, only that both are open.
