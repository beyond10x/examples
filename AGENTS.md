# AGENTS.md — examples

What this repository is for is in [README.md](README.md); this file is what an agent changing it must
know.

## State

One demo, [`zendesk-triage/`](zendesk-triage/README.md): a local support-ticket triage. Zendesk reads
go through Connectors, Loom runs the triage, and the governor checks each step against the ELS
protocol `support.triage/1` with Canon. Writes are recorded as proposed effects and never sent.

## Serves

This repository advances these objectives from `atlas/ROADMAP.md`:

- O1: the triage reads only through Connectors, the governor checks every step against `support.triage/1`, and writes stay proposed effects.
- O2: the triage's domain is an ESS specification and its protocol is data, with an ESS conformance report as the evidence for each run.

## Commands

| Command | What it does |
|---|---|
| `task check` | The gate: `cargo fmt --check`, clippy `-D warnings`, `cargo test`, `task ess`, `task conform`, `task pins`. CI runs it as `examples / task check`. |
| `task ess` | `ess specify validate --path zendesk-triage/ess`. |
| `task conform` | Compiles the authored scenarios (`ess verify conform author`), runs them with `zendesk-triage conform` against the fixture Zendesk, and writes the report (`ess verify conform report`). Output goes to `$CARGO_TARGET_DIR/zendesk-triage-ess/`. |
| `task pins` | `zendesk-triage pins`: each vendored file against its pin; `zendesk-triage bindings`: the composition against the protocol and Connectors. |
| `task demo` | The demo run the README shows. |

The Taskfile sets `CARGO_TARGET_DIR` to `$HOME/.cache/b10x-target/examples` unless one is set; give
each worktree its own before trusting a gate run there. Requires go-task and `ess` 0.52.

## Pins

Every dependency on another beyond10x repository is an exact commit or tag; `Cargo.lock` holds the
rest. Canon is named the way Loom and ELS name it (`branch = "main"`), so one Canon is built; the
lock fixes it.

| Dependency | Pin | Where |
|---|---|---|
| Loom (`b10x-loom-sdk`: Commission's contracts and runtime, the Loom executor, the governor) | `c4e7252e68e86d83182e7fe9615f931d30c3d96f` | `zendesk-triage/Cargo.toml` |
| Canon (`b10x-canon`) | `branch = "main"`, locked at `d2e09aeeb3126dad820585785db608d1737dc643`, the commit Loom locks | `Cargo.lock` |
| ELS (`b10x-canon-engineering`, used only by the pin check) | `59fd19f639a6427533cd63c5ca53eaaf040d70c7` | `zendesk-triage/Cargo.toml` |
| `support.triage/1`, vendored | ELS `59fd19f639a6427533cd63c5ca53eaaf040d70c7`, SHA-256 `d970d928…c67a` | `vendor/els/PIN.yaml` |
| Connectors (`connectors-catalog-provider`, `-catalog`, `-core`, `-sdk`) | tag `v0.26.0` (`b59460ddbe1d7d021e240d3a3e39afd0e6379986`) | `zendesk-triage/Cargo.toml` |
| Zendesk bundle and selection set, vendored | Connectors `v0.26.0` | `zendesk-triage/connectors/PIN.yaml` |
| ess CLI | 0.52.0 | `.github/workflows/check.yml`, `ess-inputs.yaml` |

Governor and Commission are not separate dependencies. Atlas ADR 0090 (accepted 2026-10-05) moved
both into Loom: `beyond10x/governor` main `81fcc1e` and `beyond10x/commission` main `e61e4f0` are
the code Loom's `loom-governor` and `loom-commission` crates carry, and depending on those
repositories beside Loom would build a second, incompatible Commission.

### Switching the protocol to a released ELS

Loom's governor reads protocols from the ELS registry, and Loom pins `b10x-els` at els `ac7dd03`,
which predates `support.triage/1`. The root `Cargo.toml` `[patch."https://github.com/beyond10x/els"]`
replaces that crate with `vendor/els`, a stand-in registry serving the vendored copy. That patch is
the one place to switch:

1. When Loom pins an ELS that carries `support-triage/1.yaml`, delete the `[patch]` section and
   `vendor/els/`, and drop the `b10x-els` dependency from `zendesk-triage/Cargo.toml`.
2. Until then, to move to a newer ELS commit: copy `protocols/support-triage/1.yaml` from it into
   `vendor/els/protocols/support-triage/1.yaml`, and set that commit in `vendor/els/PIN.yaml`,
   `vendor/els/src/lib.rs` (`SOURCE_COMMIT`) and the `b10x-canon-engineering` `rev` in
   `zendesk-triage/Cargo.toml`. `task pins` refuses until all four agree byte for byte.

## Rules

- Anything that runs is Rust, with clap derive for command lines.
- Each demo is runnable product code: its domain is specified in ESS (`<demo>/ess/`) before stories
  are written around it. The triage's suite is authored scenarios only, run by the demo's own
  runner (`zendesk-triage conform`); ESS writes the report from its results
  (`external-scenario-status/1`).
- Protocol action names are tool-agnostic. The binding of an action to a tool lives in the demo's
  `composition.yaml`, never in the protocol.
- No write is sent to an external system from a demo: writes are proposed effects.
- Fixtures are synthetic: example.com addresses, no real names, no real tickets.
- No company or customer names, no credentials, no `/home/<name>/` path literals.
- Every commit and push is `b10x-bot[bot]`'s through `b10x-gates bot`; every GitHub write goes
  through `b10x-gates api`.
- Use a managed worktree for changes.
- Documentation: an independent site later, or none; never the unified Website.
