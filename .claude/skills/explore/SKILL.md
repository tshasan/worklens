---
name: explore
description: Exploratory testing of worklens as a user. Use when asked to explore, dogfood, drive or fuzz the TUI or CLI by hand, try a first run, or hunt for UI breakages the proptests don't catch.
---

# Explore worklens

A session-based exploratory test: you play a **persona** on one **charter**, read every frame the way that person would, and turn each finding into a **red** test. The proptests already prove nothing panics; you look for screens that are wrong, misleading, or ugly.

## 1. Pick a charter

A charter is one persona × one setup × one size. Take the one you were given, or the first in this table nobody has run on this branch:

| Persona | Setup (`--cache`) | Sizes | Mission |
|---|---|---|---|
| First-time user, no keys | `bugzilla_only.json` | 80x24, 60x20 | Every message names what to do next; nothing claims a failure |
| Overloaded engineer | `busy.json --whiteboard [aiplatform]` | 80x24, 200x50 | Every section, menu and detail tab stays readable and aligned |
| Damaged cache | `raw.json --mutate N`, five seeds that parse | 80x24 | Odd data degrades to honest unknowns, never to wrong facts |
| Agent | the CLI with `--json` and `--compact` | n/a | Every field an agent reads agrees with the text output and the skill |
| Tiny terminal | `busy.json` | 40x12, 20x8 | Something sensible draws; keys still work |

Done when you have written the charter at the top of your notes.

## 2. Drive it

Both tools live on this branch; if either is missing, you're on an older base than the one you were asked to test.

**TUI**, headless, no network, clock fixed at 2026-11-03T20:00Z, effects printed (`>>`) and never run:

```sh
cargo run -q -p worklens-tui --example drive -- --help
cargo run -q -p worklens-tui --example drive -- --cache busy.json --whiteboard '[aiplatform]' --section sprint --size 80x24 down enter
```

**CLI**, in a throwaway home (`scripts/sandbox --help`). Cached commands read a fixture; `--replay` answers the network from recorded files, so `refresh` and `doctor` run offline:

```sh
scripts/sandbox --cache busy.json --config '{"whiteboard":"[aiplatform]"}' -- sprint --cached
scripts/sandbox --config '{"email":"me@mozilla.com"}' --replay crates/worklens/tests/replay/first_run -- doctor
```

The CLI uses the wall clock, so ages and countdowns differ from drive by the days since 2026-11-03. Compare wording and structure across the two, not the numbers. Sources checks and action feedback ("Copied brief for D123") come from the binary; judge them through `doctor` and the CLI commands.

Cover every section, every detail tab, the actions menu, help, Sources and Settings, filter, done and snooze, and each CLI command in `worklens --help`. Done when each of those has been reached at least once for this charter.

## 3. Judge each frame

Read every frame against these oracles. `!!` lines are `testkit::screen_problems` already flagging one.

- **Honest**: a section that's off says so and says how to turn it on. Only a real failure reads as a failure. Reasons state checkable facts ("D123 accepted by quillfox, 9 days").
- **Signpost**: names what is wrong and where to fix it (a command, a URL); worklens never fixes it in place.
- **Consistent**: the TUI, the text CLI, `--json` and MCP tell the same story with the same words. `--limit` and similar flags act on every output.
- **Legible** (`AESTHETICS.md`): aligned columns, wrapped text under its own label, truncation with `…` and a gap after it, numbers with units, no bare ID that reads like a count, no line printed twice.
- **Stable**: layout doesn't jump between loading and loaded, or across resizes; the selection survives.

Done when every frame you produced has been read against every oracle.

## 4. Make each finding red

Every test you add must be cheat-proof by the rules in `AGENTS.md` (Hunting bugs). For each finding, in this order of preference:

1. **An invariant** in `testkit::screen_problems` (`crates/testkit/src/lib.rs`) when the rule holds on every screen. The TUI proptest checks it after each random step, so the fuzzer hunts the whole class from then on. Put the screen you saw in the `lies` list in `crates/testkit/tests/oracle.rs` first and watch it fail.
2. **A render test** in `crates/tui/tests/render.rs`, or **a CLI test** in `crates/worklens/tests/cli.rs`, for one screen or command.
3. **A fixture case** in `busy.json` or a new fixture added to `testkit::CACHES`, when the bug needs data no seed has.

Then run `PROPTEST_CASES=20000 cargo test --profile hunt -p worklens-tui --test robust`. When it flags an honest screen, the oracle is wrong: add that screen to the `honest` list in `oracle.rs` and change `screen_problems` until both lists pass.

Prove each test red: break the source the way a real regression would (or write the test before the fix), watch it fail for the reason you found, restore, watch it pass. Fix product code only when the user asked for fixes; commit any `robust.regressions` lines the fuzzer writes along with the fix.

Done when every finding has a test name and you've seen each one fail.

## 5. Report

One row per finding: what the persona saw, the oracle it breaks, the test that now catches it, and fixed or open. Then list what the charter didn't reach.
