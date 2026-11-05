# worklens

A terminal view of your Mozilla Firefox work across Bugzilla, Phabricator, Lando
and Jira, organized by whose turn it is. It glues these tools together rather
than replacing them: it shows where they disagree and links to the place to fix
it. It only reads from them. GitHub is not supported yet.

## Install

Build it from a clone:

```sh
git clone https://github.com/tshasan/worklens
cd worklens
cargo install --locked --path crates/worklens
```

Needs Rust 1.97+ (`rustup update`). Run the last command again after `git pull`
to update.

## Setup

You need a key for each service you use:

- Phabricator: a [Conduit token](https://phabricator.services.mozilla.com/settings/).
  If you have `~/.arcrc`, setup uses that.
- Jira: an [Atlassian API token](https://id.atlassian.com/manage-profile/security/api-tokens).
- Bugzilla: an [API key](https://bugzilla.mozilla.org/userprefs.cgi?tab=apikey).
  You only need this to see restricted bugs.

Then run:

```sh
worklens setup
worklens doctor
```

`setup` asks for each key and checks it. It saves keys in your OS keychain and
other settings in `~/.config/worklens.json`. It also asks for your team's
whiteboard tag and your sprint budget. Press Enter to skip a question.

`doctor` checks each service and tells you how to fix anything that fails.

## Use

Run `worklens` to open the TUI. The first run takes a few seconds to fetch your
data; later runs open the cache at once.

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
worklens show D123        # one bug, revision or Jira ticket
worklens brief 2066590    # a markdown summary to hand to an agent
```

- `--json` or `--compact` prints JSON instead of text.
- `--fresh` fetches before printing; `--cached` never fetches.
- `worklens help <command>` describes any command.

To change a key later, use `worklens auth set jira` (or `bugzilla`,
`phabricator`). `worklens auth status` shows where each key comes from.

## Agents

- Claude Code: `ln -s "$PWD/skills/worklens" ~/.claude/skills/worklens`, run
  from a clone of this repo.
- MCP clients: `worklens mcp` (stdio).
- Anything else: `worklens agent-instructions >> AGENTS.md`.

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
