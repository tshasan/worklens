# worklens

Rust workspace. CI runs all three; they must pass:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                       # no network
```

`cargo install --path crates/worklens` updates the `worklens` on PATH.

The user is learning Rust. Prefer indexes over lifetimes, enums over traits,
plain functions over async. No comments.

## Layout

Data flows one way: `sources → raw (cached) → core::build_snapshot → cli | tui | mcp`.

```
crates/core/     pure: no I/O, async or clock (take `now`); clippy.toml enforces it
crates/sources/  one module per API: thin fetch, pure parse, fixture tests
crates/store/    cache and marks files, atomic writes
crates/tui/      app.rs update(msg) -> effects; view.rs draws; keys.rs BINDINGS
crates/worklens/ binary: CLI, event loop, MCP, JSON output
skills/worklens/ Claude Code skill, also printed by `worklens agent-instructions`
```

## Hunting bugs

`crates/testkit` mutates fixture JSON (nasty strings, edge numbers, removed
keys, wrong types) for proptest. `tests/robust.rs` in core, sources, store,
tui and worklens feed it to every parser, `build_snapshot`, the cache round
trip, random key/resize/fetch sequences drawn after each step, the MCP server,
and the CLI with a damaged config or cache. They run in `cargo test`; go
deeper locally or in the nightly `hunt` workflow:

```sh
PROPTEST_CASES=20000 cargo test --profile hunt --workspace --test robust
cargo test --profile hunt -p worklens-tui --test frame_budget -- --ignored --nocapture
cargo mutants --in-diff <(git diff main)    # did the tests notice when the code broke?
```

For what invariants can't judge (misleading words, layout), run the `explore`
skill (`.claude/skills/explore`): it drives the TUI headlessly with
`cargo run -p worklens-tui --example drive` and turns findings into tests.

Seeds are `store/fixtures/raw.json`, `bugzilla_only.json` (a first run with
no Jira or Phabricator) and `busy.json`, an overloaded week that
`core/tests/busy.rs` holds to filling every section and firing every rule. A
new rule or section needs a case in `busy.json` or that test fails.

A test counts only if it's cheat-proof:

- It runs the real code. Fake only the network, never worklens's own code:
  `WORKLENS_REPLAY=<dir>` makes `Http` answer from `<dir>/routes.json` and
  fail loudly on anything unrecorded (`crates/worklens/tests/replay/`), and
  `scripts/sandbox` runs the binary in a throwaway home.
- Its expected value comes from the fixture or the spec, never from the
  function under test (judge "Jira is off" from `raw.errors`, not
  `Raw::state`).
- You have seen it go red by breaking the source, not the test, and
  `cargo mutants --in-diff` finds no surviving mutant it should have caught.
- `crates/testkit/tests/oracle.rs` pins the screens `screen_problems` must
  catch and must pass. Add to both lists; never delete or loosen an entry.

A failing case is saved to `crates/*/tests/robust.regressions`; commit it with the
fix so it replays forever. `hunt` keeps overflow checks on; plain `--release`
hides them. Each update plus redraw must fit a 16 ms frame with 1500 items.

## Rules

- Section and reason: in `core/src/rules.rs` add a `Reason` variant, a row in
  `RULES`, its `Facets` (tag, legend gloss, landed-only, sort) and its
  `next_step`; then its text in `ReasonText` (`snapshot.rs`). The help legend
  reads `rules::TAGS`. Reasons state checkable facts ("D123 accepted by
  quillfox, 9 days"). Thresholds come from Mozilla (BugBot, BMO).
- New API field: add to `raw.rs` with `#[serde(default)]`, parse with a fixture
  test, bump `RAW_VERSION` if old caches break.
- `--json`, `--compact` and MCP output (`json.rs`, schema "worklens/2"): add
  fields freely; renaming or removing one breaks agents. Update the skill.
- New list key: a row in `BINDINGS`; help must fit 80x24 (`tui/tests/render.rs`).
  Prefer arrows, enter, esc and the actions menu over new letters.
- Lint exceptions: `#[expect(clippy::lint, reason = "...")]`, never `allow`.
- Read-only toward Bugzilla, Jira and Phabricator unless a change asks for writes.
- TUI design: `AESTHETICS.md`.
