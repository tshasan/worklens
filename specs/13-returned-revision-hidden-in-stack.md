# Changes requested on a stacked revision hidden behind "chk"

Status: implemented 2026-11-19. Found 2026-11-19 comparing `worklens sections` with Phabricator.

## Problem

When a bug has several revisions in different states, worklens summarises
the bug by one of them. A revision with changes requested can end up hidden
behind an accepted sibling, so it never appears in "Returned to you".

## Evidence

Bug 2092358 has three revisions:

- D342404 accepted
- D342405 needs-revision ("Continue Smart Window chat after a user clears a captcha")
- D342406 needs-review

Worklens showed the bug only as `chk` ("D342404 depends on D338974, which is
needs-revision; try failed"). D342405 did not appear under "Returned to you",
though it is my turn on it.

## Expected

- Any revision I authored in needs-revision shows in "Returned to you",
  whatever state its siblings on the same bug are in.
- The bug-level summary names every revision that needs me, not only the
  first or the accepted one.
