---
name: worklens
description: The user's Mozilla work across Bugzilla, Jira and Phabricator, organized by whose turn it is. Use when the user asks what to work on next, what's on their plate, about their sprint or story points, needinfos or reviews waiting on them, whether Bugzilla and Jira are in sync, which Jira Product Discovery ideas (AIDI-79) their work feeds and whether those ideas are on track, or before starting work on a specific bug, Jira ticket (AIPLAT-123) or revision (D123).
---

`worklens` is a read-only CLI over the user's Bugzilla, Jira and Phabricator, cached 10 minutes.

| Need | Command |
|---|---|
| What's waiting on the user, by section | `worklens sections --compact` |
| What to work on next | `worklens next --json` |
| Sprint points vs budget | `worklens sprint --json` |
| Bugzilla/Jira mismatches, and ideas whose target or status the work contradicts (add `--team` for the team) | `worklens drift --json` |
| Ideas the user's work rolls up to | `worklens sections --json`, items with `section: "ideas"` |
| One item | `worklens show <ref> --json` |
| Context before starting work | `worklens brief <ref>` |
| A factual engineering update for an idea's owner | `worklens brief AIDI-79` |
| Glean definitions and expiry | `worklens telemetry --json` |
| Which sources answered | `worklens health --json` |

`<ref>` is a bug number, Jira key (`AIPLAT-1305` or idea `AIDI-79`) or revision (`D321987`), or a Bugzilla, Jira or Phabricator link to one. `--fresh` skips the cache.

- Go section by section, in the order given. Repeat each item's `reason` and suggest its `action`; don't reorder. `stale: true` means past Mozilla's 7-day threshold.
- `chk` means an accepted revision needs attention before landing: classification, outstanding blocking review, an unready stack parent, or failing, running or outdated try jobs. Follow its `action`; acceptance alone is not readiness.
- Items include `dependencies`: immediate Bugzilla dependencies and Phabricator parents, with `id`, `title`, `status`, `owner`, `url`, `source`, and nullable `blocked`. Unknown means not fetched, unavailable, or not visible. An open bug dependency is context, not proof that Lando forbids landing. Accepted parents may land together in a stack. Deeper ancestors are not fetched.
- `actionUrl`, when present, opens the destination for the suggested landing or blocker action. The TUI offers it first in the actions menu and uses it for `o`.
- Mention a failing `try` push before calling a revision ready to land.
- On `isNew` items, lead with `changes` when asked what's new.
- Changes include diff and try-push identity, job results, uplift approvals, release flags, integration observations and dependency status. Failed lookups are not evidence that an approval, dependency or landing disappeared. Older saved marks establish these additional baselines without making unchanged work new.
- `show --json` has `timeline` (dated events from each tool, oldest first) and `ships` (the release Bugzilla's status flags put the fix in). Use them for "where is this" and "when does it ship"; review decisions and uplift approvals have no dates there.
- `next` returns at most `--limit` items (10 by default); `more` counts the ones left out. When it's above 0, say so or rerun with `--all`.
- If `partial` is true, a source failed: name it and when it last answered (`stale`) instead of reporting the list as complete.
- `sourceStates` maps `bugzilla`, `jira` and `phabricator` to `ok`, `off` (never set up) or `failed`. An `off` source isn't an error: say its sections are off and that `worklens auth set <source>` turns it on. Never call its empty sections "nothing to do".
- `workflow` evidence is what each source reports, not proof: Bugzilla "fixed" doesn't mean shipped, accepted review isn't every approval, and `unknown` or missing means unknown.
- Items carry `parent` (usually the epic) and `idea`, the Jira Product Discovery idea the work rolls up to: management's status, `weeklyStatus`, `target` release and owner. An idea item lists the user's items under it in `idea.work`; its drift has no `fix` because the idea belongs to its owner. Drift also flags work under a closed epic. With `--team`, ideas appear when the team's open bugs contradict them (Released, or target passed).
- Offer `worklens brief <idea>` when the user needs to report status to a PM; it covers only the user's own items, and says so.
- Don't mention done or snoozed items unless asked. Don't change Bugzilla or Jira for the user.
- Auth error: run `worklens doctor` and show the result. Nothing set up: ask the user to run `worklens setup` in a terminal. Never ask for or print a key.
