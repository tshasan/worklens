# Needinfo on a resolved bug

Status: implemented 2026-11-19. Found 2026-11-19 comparing `worklens sections` with Bugzilla.

## Problem

An open needinfo on a bug that is already RESOLVED FIXED ranks first in
"Waiting on you", as if it were the most urgent thing I have. In practice
the question was answered by landing the fix and the flag was just never
cleared.

## Evidence

- Bug 2086888 (metric expiry nag): needinfo from aryx, 21 days. The fix
  landed and the bug was resolved the same day worklens showed it at the top.
- Bug 2092311 (intermittent): needinfo from the release-mgmt bot, 6 days.
  Bug resolved the same day.

Both had `needinfo?` with me as requestee and `status: RESOLVED`.

## Expected

- A needinfo on a closed bug is not ranked as critical. It belongs lower
  (or in its own "clear these flags" group) with a reason such as
  "bug resolved FIXED on <date>; needinfo still set".
- Needinfos on open bugs keep their current placement.
- Bot-set needinfos (release-mgmt, intermittent bots) on resolved bugs are
  the most common case and should never outrank human review requests.
