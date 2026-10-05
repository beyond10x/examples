# AGENTS.md — examples

What this repository is for is in [README.md](README.md); this file is what an agent changing it must
know.

## State

Empty apart from this bootstrap. The first demo (a local support-ticket triage over connectors,
run by Loom and governed by an engineering protocol) arrives by a bot pull request.

## Rules

- Anything that runs is Rust, with clap derive for command lines.
- Each demo is runnable product code: its domain is specified in ESS before stories are written
  around it.
- No company or customer names, no credentials, no `/home/<name>/` path literals.
- Every commit and push is `b10x-bot[bot]`'s through `b10x-gates bot`; every GitHub write goes
  through `b10x-gates api`.
- Use a managed worktree for changes.
- Documentation: an independent site later, or none; never the unified Website.
