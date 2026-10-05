//! One triage: a case on `support.triage/1` for one ticket, run by Commission's runtime with Loom
//! as executor until the governor holds an outcome legitimate or the run stops.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use loom_sdk::commission::model::behaviour::Generated;
use loom_sdk::commission::model::json::Value as Model;
use loom_sdk::commission::model::primitives::Timestamp;
use loom_sdk::commission::model::responsibility::{
    ActionRequestId, AgentRevisionId, AuthorityContext, CaseId, Commission, CommissionData,
    CommissionId, ObservationId, PrincipalId, RevalidateActionRequestOutcome, RunId, RunOutcome,
    Truth, commission_state,
};
use loom_sdk::commission::outcome::RunStore;
use loom_sdk::commission::ports::governor::Governor;
use loom_sdk::{CanonGovernor, Loom, LoopContext, MemoryCaseStore, run_until_blocked};

use crate::agent::{RuleArguments, RuleSelector};
use crate::authority::PolicyAuthority;
use crate::composition::Composition;
use crate::effects::{
    Classification, Ids, Ledger, ProposedEffect, Step, TriageEffects, kind, lock,
};
use crate::protocol::PROTOCOL;
use crate::zendesk::ZendeskReads;

/// The instant the fixture records are read at: 2026-10-05T10:00:00Z.
pub const FIXTURE_NOW: i64 = 1_791_194_400;

/// The most steps one run takes before the runtime suspends it.
pub const STEP_BUDGET: usize = 16;

/// How a run is made.
#[derive(Debug, Clone)]
pub struct Options {
    /// The capabilities the operator approves for the run.
    pub approvals: BTreeSet<String>,
    /// The instant the run reads as now, Unix seconds.
    pub now: i64,
}

/// How a triage ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Triaged,
    Escalated,
    /// `requester_unidentified` or `classification_rejected`.
    NeedsHuman {
        reason: String,
    },
    /// The actions waiting for a capability a person must approve.
    AwaitingApproval {
        actions: Vec<String>,
    },
    /// The ticket read answered not found.
    UnknownTicket,
    /// Anything else, with the runtime's outcome.
    Stopped {
        reason: String,
    },
}

impl std::fmt::Display for Ended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Triaged => f.write_str("triaged"),
            Self::Escalated => f.write_str("escalated"),
            Self::NeedsHuman { reason } => write!(f, "needs_human ({reason})"),
            Self::AwaitingApproval { actions } => {
                write!(f, "awaiting approval ({})", actions.join(", "))
            }
            Self::UnknownTicket => f.write_str("refused: ticket not found"),
            Self::Stopped { reason } => write!(f, "stopped: {reason}"),
        }
    }
}

/// One proposal the runtime revalidated, and what came of it.
#[derive(Debug, Clone)]
pub struct Row {
    pub action: String,
    /// The governor's answer through the runtime's revalidation.
    pub governor: String,
    /// The authority provider's answer, when the frontier named a capability.
    pub authority: Option<String>,
    /// What the effect port did, when the request was admitted.
    pub effect: Option<Step>,
}

/// The decision trail of one triage.
#[derive(Debug, Clone)]
pub struct Triage {
    pub ticket: String,
    pub case: String,
    pub transport: String,
    pub ended: Ended,
    pub rows: Vec<Row>,
    /// Every claim of the protocol and its value after the run.
    pub claims: Vec<(String, String)>,
    /// Every obligation and whether it is still open.
    pub obligations: Vec<(String, bool)>,
    pub classification: Option<Classification>,
    pub review: Option<String>,
    pub proposed: Vec<ProposedEffect>,
}

impl Triage {
    /// The value of `claim` after the run.
    pub fn claim(&self, claim: &str) -> Option<&str> {
        self.claims
            .iter()
            .find(|(name, _)| name == claim)
            .map(|(_, value)| value.as_str())
    }

    /// The queue the run proposed to route to.
    pub fn route(&self) -> Option<&str> {
        self.target_of("ticket.route")
    }

    /// The target the run proposed to escalate to.
    pub fn escalation(&self) -> Option<&str> {
        self.target_of("escalation.request")
    }

    fn target_of(&self, action: &str) -> Option<&str> {
        self.proposed
            .iter()
            .find(|effect| effect.action == action)
            .map(|effect| effect.target.as_str())
    }
}

/// What the loop needs that no model may supply: ids, the time and the step budget.
struct Context {
    ids: Arc<Ids>,
    now: i64,
}

impl LoopContext for Context {
    fn action_request_id(&mut self) -> ActionRequestId {
        ActionRequestId(self.ids.next(kind::ACTION_REQUEST))
    }

    fn observation_id(&mut self) -> ObservationId {
        ObservationId(self.ids.next(kind::OBSERVATION))
    }

    fn now(&mut self) -> Timestamp {
        Timestamp(crate::time::format(self.now))
    }

    fn step_budget(&self) -> Option<usize> {
        Some(STEP_BUDGET)
    }
}

/// Triages ticket `ticket` (its numeric id) with `reads`, under `composition`.
///
/// # Errors
/// When the ticket id is not a number, the composition does not fit the protocol or the connector,
/// the governor does not open the case, or the runtime fails without an outcome.
pub fn run(
    reads: &dyn ZendeskReads,
    composition: &Composition,
    ticket: &str,
    options: &Options,
) -> Result<Triage, String> {
    let ticket_id: u64 = ticket
        .parse()
        .ok()
        .filter(|_| ticket.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| format!("ticket id `{ticket}` is not a number"))?;
    let problems = composition.problems(&crate::protocol::compiled()?, &reads.operations());
    if !problems.is_empty() {
        return Err(format!(
            "the composition does not fit:\n  {}",
            problems.join("\n  ")
        ));
    }

    let governor = CanonGovernor::new(MemoryCaseStore::default());
    let case = CaseId(format!("tri-{ticket_id}"));
    let revisions: BTreeMap<String, String> = [("ticket", "r0"), ("requester", "r0")]
        .into_iter()
        .map(|(artifact, revision)| (artifact.to_owned(), revision.to_owned()))
        .collect();
    governor
        .open_case(case.clone(), PROTOCOL, revisions)
        .map_err(|error| error.to_string())?;

    let ids = Arc::new(Ids::default());
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let loom = Loom::new(
        RuleSelector::new(ledger.clone()),
        RuleArguments::new(ledger.clone(), composition),
        format!("triage ticket {ticket_id}"),
    );
    let authority = PolicyAuthority::new(composition, options.approvals.clone());
    let effects = TriageEffects {
        governor: &governor,
        case: case.clone(),
        ticket_id,
        reads,
        composition,
        ledger: ledger.clone(),
        ids: ids.clone(),
        now: options.now,
    };
    let run_ids = ids.clone();
    let mut runs = Generated::new(RunStore::new(move || RunId(run_ids.next(kind::RUN))));
    let commission = Commission::<commission_state::Assigned>::new(CommissionData {
        commission_id: CommissionId(ids.next(kind::COMMISSION)),
        agent_revision_id: AgentRevisionId(ids.next(kind::AGENT_REVISION)),
        case_id: case.clone(),
        principal: PrincipalId("operator".to_owned()),
        authority_context: AuthorityContext(Model::Null),
    });
    let end = run_until_blocked(
        &governor,
        &loom,
        &authority,
        &effects,
        &commission,
        &mut runs,
        &mut Context {
            ids: ids.clone(),
            now: options.now,
        },
    )
    .map_err(|error| error.to_string())?;
    drop(effects);

    let frontier = governor
        .frontier(&case)
        .map_err(|error| format!("{error:?}"))?
        .into_data();
    let claims: Vec<(String, String)> = frontier
        .claims
        .iter()
        .map(|claim| {
            let value = match claim.value {
                Truth::True => "true",
                Truth::False => "false",
                Truth::Unknown => "unknown",
            };
            (claim.claim.clone(), value.to_owned())
        })
        .collect();
    let obligations = frontier
        .obligations
        .iter()
        .map(|obligation| (obligation.obligation.clone(), obligation.open))
        .collect();

    let ledger = std::mem::take(&mut *lock(&ledger));
    let mut decisions = authority.decisions().into_iter();
    let mut awaiting = Vec::new();
    let rows = end
        .requests
        .iter()
        .map(|made| {
            let (governor, authority) = match &made.outcome {
                RevalidateActionRequestOutcome::Admitted => ("admissible".to_owned(), None),
                RevalidateActionRequestOutcome::NeedsAuthority { error } => {
                    let decision = decisions.next().map(|d| d.verdict);
                    if decision.as_deref() == Some("approval required") {
                        awaiting.push(error.action.clone());
                    }
                    (
                        format!("approval-required ({})", error.capability),
                        decision,
                    )
                }
                RevalidateActionRequestOutcome::NotAdmitted { error } => {
                    (format!("refused: {}", error.reasons.join("; ")), None)
                }
                RevalidateActionRequestOutcome::Stale { .. } => ("stale".to_owned(), None),
            };
            Row {
                action: made.request.action.clone(),
                governor,
                authority,
                effect: ledger
                    .steps
                    .iter()
                    .find(|step| step.action_request == made.request.action_request_id.0.0)
                    .cloned(),
            }
        })
        .collect();

    let claim_is = |name: &str, value: &str| {
        claims
            .iter()
            .any(|(claim, held)| claim == name && held == value)
    };
    let ended = if ledger.not_found {
        Ended::UnknownTicket
    } else {
        match &end.outcome {
            RunOutcome::Completed(completed) => match completed.outcome.as_str() {
                "triaged" => Ended::Triaged,
                "escalated" => Ended::Escalated,
                "needs_human" => Ended::NeedsHuman {
                    reason: if claim_is("requester.known", "false") {
                        "requester_unidentified".to_owned()
                    } else {
                        "classification_rejected".to_owned()
                    },
                },
                other => Ended::Stopped {
                    reason: format!("outcome `{other}`"),
                },
            },
            RunOutcome::NeedsAuthority(_) => Ended::AwaitingApproval { actions: awaiting },
            RunOutcome::AwaitingApproval(waiting) => Ended::AwaitingApproval {
                actions: waiting.actions.clone(),
            },
            other => Ended::Stopped {
                reason: format!("{other:?}"),
            },
        }
    };

    Ok(Triage {
        ticket: ticket_id.to_string(),
        case: case.0,
        transport: reads.transport(),
        ended,
        rows,
        claims,
        obligations,
        classification: ledger.classification,
        review: ledger.review,
        proposed: ledger.proposed,
    })
}
