# examples

Runnable end-to-end demos that compose the beyond10x stack: each demo wires real components
together (connectors, Loom, the governor, engineering protocols, llm) and runs on one machine.

Each demo lives in its own directory with its own README, its ESS specification and a single
command that runs it.

| Demo | What it shows |
|---|---|
| [`zendesk-triage`](zendesk-triage/README.md) | A support ticket triaged under rules: Zendesk read through Connectors, Loom running the triage, the governor checking every step against the `support.triage/1` protocol with Canon. Runs on fixtures with no account and no model key. |

## Run

Needs Rust (stable), [go-task](https://taskfile.dev) and [`ess`](https://github.com/beyond10x/ess)
0.52 for the gate.

```console
task demo     # the zendesk-triage demo on its fixtures
task check    # format, lint, tests, ESS, conformance and pin checks
```

Licensed under Apache-2.0 ([LICENSE](LICENSE)).
