# Anonymous Bugzilla access passes doctor silently

Status: implemented 2026-11-19. Found 2026-11-19 running `worklens doctor`.

## Problem

`worklens doctor` shows a green check for Bugzilla when it runs anonymously.
Anonymous access hides restricted bugs (security, some Smart Window and
legal work), so every section can be missing items without any sign of it.

## Evidence

```
✓ bugzilla  reachable, anonymous (public bugs only; `worklens auth set bugzilla` for restricted bugs)
```

The hint is there, but it's a check mark and the main views never mention
it.

## Expected

- Doctor marks anonymous Bugzilla as a warning, not a pass.
- The header of `sections` / the TUI says "Bugzilla: public bugs only" while
  anonymous, so the lists aren't read as complete.
