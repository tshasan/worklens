# worklens

Your Firefox work is split across Bugzilla, Phabricator, Lando and Jira, and
none of them tells you which bugs are waiting on you. worklens reads all four
and sorts your work by whose turn it is.

worklens glues these tools together and leaves the editing to them. If a bug
is RESOLVED and its Jira ticket still says In Progress, worklens shows you both
and links to the ticket so you can fix it there. It only reads from these
tools.

```
 worklens   Sprint  ▓▓▓░░░░░░░  3 of 10 pts · 13 days left   Beta merge in 2 days   Updated 19m ago
 Waiting on you 1   Sprint 1
╭━━━━━━━━━━━━━━━━──────────────────────────────────────────────────────────────────────────────────╮
│>4d    ni?  ✗   1995008 Support view-source: for the PageExtractor                                │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
 Overview   Timeline   Comments   Links                                                       Tab ⇥
╭━━━━━━━━──────────────────────────────────────────────────────────────────────────────────────────╮
│ Support view-source: for the PageExtractor                                                       │
│ needinfo from a@mozilla.com, 4 days                                                              │
│ Next: Answer the needinfo                                                                        │
│                                                                                                  │
│ Bugzilla     1995008 · ASSIGNED · P3 · Core :: Machine Learning: On Device                       │
│ Jira         AIPLAT-1416 · ENG REVIEW · 3 pts · 162 Core Nov 2 - Nov 16                          │
│ Phabricator  D321987 · needs-review                                                              │
│   Reviewers  waiting on quillfox (blocking)                                                      │
│   Try        builds passed · lint passed · tests failed · 1 failing job · pushed 12d ago         │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
 ↑↓ move · ←→ section · Tab detail · Enter actions · / filter · ? help
```

## Start

```sh
git clone https://github.com/tshasan/worklens
cd worklens
cargo install --locked --path crates/worklens
worklens setup
worklens next
```

If you use `moz-phab` or `arc`, setup takes your token from `~/.arcrc` and
you can press Enter through the rest. Public bugs need no Bugzilla key, so you
can start without creating one.

`worklens next` prints what to work on next. Run `worklens` for the full view.

You need Rust 1.97+ (`rustup update`). There are no releases; to update, run
`git pull` and then the `cargo install` line again.

## Keys

Each key is optional. Without one, worklens skips that service and says so.

| Service     | Key                                                                          | Without it                   |
| ----------- | ---------------------------------------------------------------------------- | ---------------------------- |
| Phabricator | [Conduit token](https://phabricator.services.mozilla.com/settings/), or `~/.arcrc` | no reviews or revisions |
| Jira        | [Atlassian API token](https://id.atlassian.com/manage-profile/security/api-tokens) | no sprint or Jira drift |
| Bugzilla    | [API key](https://bugzilla.mozilla.org/userprefs.cgi?tab=apikey)             | no restricted bugs           |

`setup` asks for each key and checks it. It saves keys in your OS keychain and
other settings in `~/.config/worklens.json`. It also asks for your team's
whiteboard tag and your sprint budget. `worklens doctor` checks each service
and tells you how to fix anything that fails.

To change a key later, use `worklens auth set jira` (or `bugzilla`,
`phabricator`). `worklens auth status` shows where each key comes from.

## Use

The first run takes a few seconds to fetch your data. Later runs open from
the cache.

```
↑ ↓      move               o   open in browser
← →      switch section     /   filter
Tab      switch detail tab  r   refresh
Enter    actions            s   sources and settings
?        all keys           q   quit
```

From the shell:

```sh
worklens next             # what to work on next
worklens sections         # everything, by section
worklens sprint           # sprint points against your budget
worklens drift --team     # where Bugzilla and Jira disagree
worklens local            # local commits not on Phabricator yet
worklens show D123        # one bug, revision or Jira ticket
worklens brief 2066590    # a markdown summary to hand to an agent
```

- `--json` or `--compact` prints JSON instead of text.
- `--fresh` fetches before printing; `--cached` never fetches.
- `worklens help <command>` describes any command.

## Agents

- Claude Code: `ln -s "$PWD/skills/worklens" ~/.claude/skills/worklens`, run
  from a clone of this repo.
- MCP clients: `worklens mcp` (stdio).
- Anything else: `worklens agent-instructions >> AGENTS.md`.

## How it works

On each refresh, worklens reads Bugzilla, Phabricator, Lando and Jira and
caches the results. It links bugs, revisions and tickets through the IDs they
share and sorts them into sections by whose turn it is.

It also runs `git` in your Firefox checkout (the agent directory, `~/firefox`
by default). It looks at your commits from the last 90 days that aren't on
`origin/main`, matches each to a revision by its `Differential Revision:` line
and to a bug by `Bug 123` in the subject. Commits with no revision go under Not
uploaded. If an uploaded commit's changes or title differ from the revision's
latest diff, worklens flags it as drift; a rebase alone doesn't. It skips
branches with "backup" in the name.

worklens doesn't read GitHub yet.

## Uninstall

```sh
worklens auth remove bugzilla
worklens auth remove jira
worklens auth remove phabricator
cargo uninstall worklens
rm -r ~/.config/worklens.json ~/.cache/worklens ~/.local/state/worklens
```

## Development

See [AGENTS.md](AGENTS.md) for layout and rules, and
[AESTHETICS.md](AESTHETICS.md) for the TUI.

License: MPL-2.0.
