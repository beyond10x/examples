# Zendesk ticket triage

A support ticket triaged under rules, on one machine. The rules are the ELS protocol
`support.triage/1`; the stack runs them:

| Step | Who | What |
|---|---|---|
| Rules | ELS `support.triage/1`, evaluated by Canon | Which actions are admissible, blocked or need approval; which claims hold; which outcome is legitimate. Actions are tool-agnostic: `ticket.read`, `requester.lookup`, `ticket.classify`, `classification.review`, `ticket.route`, `reply.draft`, `escalation.request`. |
| Decide | the governor (in Loom) | Holds the case and its evidence, and answers the frontier and completion with Canon. It never acts and never grants authority. |
| Propose | Loom | Projects the frontier into a catalogue, selects one action and writes its arguments. Here a deterministic rule selector stands in for a model. |
| Check | Commission's runtime (in Loom) | Revalidates every proposal against the current frontier and asks the authority provider for any capability it names. |
| Authorize | `composition.yaml` policy | `classification.review` and `escalation.request` are allowed; `ticket.route` and `reply.draft` need the operator's `--approve`. |
| Act | the effect port, through `composition.yaml` bindings | Reads go to Zendesk through Connectors v0.26.0 (`ticket.show`, `ticket.comments`, `user.show`, `organization.show`). Route, reply and escalation are recorded as proposed effects and **never sent**. The effect port, not the agent, turns what it read into evidence. |

The case holds ids, revisions, digests, categories and records; ticket text stays in the run and
never reaches the case or the trail.

## Run it on the fixtures

No Zendesk account and no model key. The fixture is a Zendesk in the process, serving synthetic
tickets, users and organizations (example.com addresses, no real names) to the real Connectors
engine, at a fixed clock of 2026-10-05T10:00:00Z.

```console
cargo run -p zendesk-triage -- triage --approve ticket.route   # every fixture ticket
cargo run -p zendesk-triage -- triage 1001                     # without the routing approval
task demo                                                      # both, plus an unknown id
```

| Ticket | Fixture | Outcome |
|---|---|---|
| 1001 | billing question, within its first-reply target | `triaged`, routed to `queue-billing` (with `--approve ticket.route`); without it the run stops at `ticket.route` waiting for approval |
| 1002 | urgent, no reply for three hours | `escalated` to `support-oncall` |
| 1003 | requester suspended | `needs_human`: requester not identified |
| 1004 | no classification rule matches | `needs_human`: the review rejects `other` |
| 9999 | not in the fixture | refused: ticket not found |

### Output

`task demo` on 2026-10-05, fixtures, exit 0 for both runs:

```console
$ zendesk-triage triage 1001 1002 1003 1004 9999 --approve ticket.route
reads: connectors catalog engine; now 2026-10-05T10:00:00Z; approvals: ticket.route

ticket 1001  case tri-1001  protocol support-triage@1 (ELS 59fd19f, sha256 d970d928e629…)
   1. ticket.read            governor: admissible
      connectors zendesk ticket.show + ticket.comments: revision c-03d590075284, priority normal, first reply within_target
      evidence ticket_snapshot = within_target on ticket@c-03d590075284
   2. requester.lookup       governor: admissible
      connectors zendesk user.show + organization.show: requester identified, organization found
      evidence requester_profile = identified on requester@u-60241fd33f44
   3. ticket.classify        governor: admissible
      local classifier: proposed billing/normal
      evidence classification on ticket@c-03d590075284
   4. classification.review  governor: approval-required (classification.review); authority: allowed by policy
      local reviewer: approved billing/normal (local reviewer)
      evidence classification_review = approved on ticket@c-03d590075284
   5. ticket.route           governor: approval-required (ticket.route); authority: approved by the operator
      recorded, never sent
      proposed effect ticket.route -> queue-billing (not sent)
      evidence routing_record on ticket@c-03d590075284
  claims: classification.proposed=true classification.reviewed=true escalation.requested=unknown requester.known=true sla.breached=false ticket.classified=true ticket.current=true ticket.routed=true ticket.triaged=true
  obligations: escalate_sla_breach discharged
  outcome: triaged (billing/normal, route queue-billing)
  proposed effects, none sent: ticket.route -> queue-billing

ticket 1002  case tri-1002  protocol support-triage@1 (ELS 59fd19f, sha256 d970d928e629…)
   1. ticket.read            governor: admissible
      connectors zendesk ticket.show + ticket.comments: revision c-c77e088980a1, priority urgent, first reply breached
      evidence ticket_snapshot = breached on ticket@c-c77e088980a1
   2. escalation.request     governor: approval-required (escalation.request); authority: allowed by policy
      recorded, never sent
      proposed effect escalation.request -> support-oncall (not sent)
      evidence escalation_record on ticket@c-c77e088980a1
  claims: classification.proposed=unknown classification.reviewed=unknown escalation.requested=true requester.known=unknown sla.breached=true ticket.classified=unknown ticket.current=true ticket.routed=unknown ticket.triaged=unknown
  obligations: escalate_sla_breach discharged
  outcome: escalated (to support-oncall)
  proposed effects, none sent: escalation.request -> support-oncall

ticket 1003  case tri-1003  protocol support-triage@1 (ELS 59fd19f, sha256 d970d928e629…)
   1. ticket.read            governor: admissible
      connectors zendesk ticket.show + ticket.comments: revision c-1313eba803a9, priority normal, first reply within_target
      evidence ticket_snapshot = within_target on ticket@c-1313eba803a9
   2. requester.lookup       governor: admissible
      connectors zendesk user.show + organization.show: requester unidentified
      evidence requester_profile = unidentified on requester@u-74800ea57ab7
  claims: classification.proposed=unknown classification.reviewed=unknown escalation.requested=unknown requester.known=false sla.breached=false ticket.classified=unknown ticket.current=true ticket.routed=unknown ticket.triaged=false
  obligations: escalate_sla_breach discharged
  outcome: needs_human (requester_unidentified)

ticket 1004  case tri-1004  protocol support-triage@1 (ELS 59fd19f, sha256 d970d928e629…)
   1. ticket.read            governor: admissible
      connectors zendesk ticket.show + ticket.comments: revision c-818702130d21, priority low, first reply within_target
      evidence ticket_snapshot = within_target on ticket@c-818702130d21
   2. requester.lookup       governor: admissible
      connectors zendesk user.show + organization.show: requester identified, organization found
      evidence requester_profile = identified on requester@u-cbb01d693d51
   3. ticket.classify        governor: admissible
      local classifier: proposed other/low
      evidence classification on ticket@c-818702130d21
   4. classification.review  governor: approval-required (classification.review); authority: allowed by policy
      local reviewer: rejected other/low (local reviewer)
      evidence classification_review = rejected on ticket@c-818702130d21
  claims: classification.proposed=true classification.reviewed=false escalation.requested=unknown requester.known=true sla.breached=false ticket.classified=false ticket.current=true ticket.routed=unknown ticket.triaged=false
  obligations: escalate_sla_breach discharged
  outcome: needs_human (classification_rejected)

ticket 9999  case tri-9999  protocol support-triage@1 (ELS 59fd19f, sha256 d970d928e629…)
   1. ticket.read            governor: admissible
      connectors zendesk ticket.show + ticket.comments
      refused by the effect port: Zendesk has no ticket 9999
  claims: classification.proposed=unknown classification.reviewed=unknown escalation.requested=unknown requester.known=unknown sla.breached=unknown ticket.classified=unknown ticket.current=unknown ticket.routed=unknown ticket.triaged=unknown
  obligations: escalate_sla_breach open
  outcome: refused: ticket not found

summary
  ticket 1001   triaged                                  5 step(s)
  ticket 1002   escalated                                2 step(s)
  ticket 1003   needs_human (requester_unidentified)     2 step(s)
  ticket 1004   needs_human (classification_rejected)    4 step(s)
  ticket 9999   refused: ticket not found                1 step(s)
exit 0

$ zendesk-triage triage 1001
reads: connectors catalog engine; now 2026-10-05T10:00:00Z; approvals: none

ticket 1001  case tri-1001  protocol support-triage@1 (ELS 59fd19f, sha256 d970d928e629…)
   1. ticket.read            governor: admissible
      connectors zendesk ticket.show + ticket.comments: revision c-03d590075284, priority normal, first reply within_target
      evidence ticket_snapshot = within_target on ticket@c-03d590075284
   2. requester.lookup       governor: admissible
      connectors zendesk user.show + organization.show: requester identified, organization found
      evidence requester_profile = identified on requester@u-60241fd33f44
   3. ticket.classify        governor: admissible
      local classifier: proposed billing/normal
      evidence classification on ticket@c-03d590075284
   4. classification.review  governor: approval-required (classification.review); authority: allowed by policy
      local reviewer: approved billing/normal (local reviewer)
      evidence classification_review = approved on ticket@c-03d590075284
   5. ticket.route           governor: approval-required (ticket.route); authority: approval required
      not performed
  claims: classification.proposed=true classification.reviewed=true escalation.requested=unknown requester.known=true sla.breached=false ticket.classified=true ticket.current=true ticket.routed=unknown ticket.triaged=unknown
  obligations: escalate_sla_breach discharged
  outcome: awaiting approval (ticket.route)

summary
  ticket 1001   awaiting approval (ticket.route)         5 step(s)
exit 0
```

## Run it against a Zendesk sandbox

Reads only; nothing is ever written to Zendesk. This path has not been run against a live account
by this repository.

1. Build Connectors v0.26.0 and configure its catalog provider for Zendesk, with an adapter alias
   (here `zendesk`) whose permitted operations include `ticket.show`, `ticket.comments`,
   `user.show` and `organization.show`. The configuration is in Connectors'
   `docs/catalog-zendesk.md` and `docs/local-catalog-provider.md`.
2. Connect with a Zendesk API token: `connectors connections connect --adapter zendesk --profile
   zendesk.basic --credential-prompt` (account `<email>/token`). The credential stays in
   Connectors' keyring custody; this demo never sees it.
3. Triage a ticket by id:

   ```console
   cargo run -p zendesk-triage -- triage 12345 --zendesk connectors \
     --adapter zendesk --connection <connection reference> --approve ticket.route
   ```

   The run reads now from the system clock; `--now <RFC 3339>` fixes it. `--connectors-bin` names
   the `connectors` executable when it is not on `PATH`.

## The model

Selection and arguments are deterministic rules (`src/agent.rs`), so CI and the fixtures need no
model. A real model is not offered yet: Loom's model selector (`ModelSelector`) and argument
generator carry instructions and argument schemas for `software.change/1` only, so a model would be
told it is changing software. When Loom has a protocol-neutral model selector, it plugs in where
`RuleSelector` and `RuleArguments` are built in `src/triage.rs`.

## Files

| Path | What |
|---|---|
| `composition.yaml` | Bindings from each protocol action to a Connectors read, a local service or a proposed effect; the capability policy; the category, priority, queue and first-reply lists. |
| `ess/` | The ESS specification of the demo (`examples.support_triage`) and its authored conformance scenarios. |
| `fixtures/zendesk/` | The fixture Zendesk records. |
| `connectors/` | The Zendesk bundle and selection set vendored from Connectors v0.26.0, with `PIN.yaml`. |
| `../vendor/els/` | `support.triage/1` vendored from ELS `59fd19f`, served to the governor through the ELS registry; see the repository `AGENTS.md`. |

## Known gaps

Held, not worked on here.

- **Governor (Loom):** reads protocols only from the ELS registry it pins (`ac7dd03`) and passes Canon no instant, so the 15-minute `max_age` on a ticket read never expires.
- **Commission (Loom):** no runtime binding type from an action to a tool (`decision-blocker:action-operation-binding`); `composition.yaml` and the effect port stand in.
- **Executor (Loom):** no argument schema per action (`decision-blocker:action-argument-schema`) and no protocol-neutral model selector.
- **Atlas:** `composition/1` (ADR 0087) has no format yet.
- **Connectors:** no Zendesk writes in 0.26.0, so route, reply and escalation are only proposed.
- **Canon:** no reviewer-independence or evidence-order predicate, no deployment-list categories, no capability scope, no child case on escalation.
- **ESS:** the report reads execution `passed`, conformance `inconclusive` with coverage knowledge `unknown`: an `ess-conformance/4` authored suite carries no declared coverage.
