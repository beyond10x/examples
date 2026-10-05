//! The effect port: performs what Commission's runtime admitted, through the composition's binding.
//!
//! The runtime hands this port a request only after the governor's frontier and the authority
//! provider let it through. The port is the trusted integration: it reads Zendesk through
//! Connectors, decides what the read shows (revision, first-reply status, requester identified),
//! delivers an observation of it and submits the evidence the protocol's action may produce. What
//! the agent wrote in its arguments is checked against the deployment's lists, and is evidence of a
//! proposal only.
//!
//! Writes (`ticket.route`, `reply.draft`, `escalation.request`) are recorded as proposed effects in
//! the [`Ledger`] and never sent.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use loom_sdk::commission::model::primitives::{Timestamp, Uuid};
use loom_sdk::commission::model::responsibility::{
    CaseId, Commission, EffectOutcome, EffectOutcomePerformed, EffectOutcomeRefused, EvidenceData,
    EvidenceId, Observation, ObservationData, ObservationId, commission_state,
};
use loom_sdk::commission::ports::effect::{AdmittedRequest, EffectError, EffectPort};
use loom_sdk::commission::ports::evidence::{ObservationPort, submit_evidence};
use loom_sdk::commission::ports::governor::Governor;
use loom_sdk::{CanonGovernor, MemoryCaseStore};
use serde_json::{Value, json};

use crate::composition::{Binding, Composition};
use crate::json::{from_model, sha256_hex, to_model};
use crate::zendesk::{ReadError, ZendeskReads, id_input};

/// The producer every evidence record of this port is attributed to.
pub const PRODUCER: &str = "zendesk-triage/effect-port";

/// What the run learned and did, shared by the effect port, the agent and the trail. Ticket text
/// lives here for the length of the run only and is never written to the case.
#[derive(Debug, Default)]
pub struct Ledger {
    /// The ticket as last read: what the classifier reads.
    pub ticket: Option<TicketText>,
    /// The requester the last ticket read names.
    pub requester_id: Option<u64>,
    /// The last classification proposed.
    pub classification: Option<Classification>,
    /// The last review's decision.
    pub review: Option<String>,
    /// One entry per request the port was handed, in order.
    pub steps: Vec<Step>,
    /// Every write proposed, in order. None was sent.
    pub proposed: Vec<ProposedEffect>,
    /// The ticket read answered not found.
    pub not_found: bool,
}

/// The ticket text the run may read.
#[derive(Debug, Clone)]
pub struct TicketText {
    pub subject: String,
    pub description: String,
    pub priority: Option<String>,
}

/// A proposed category and priority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub category: String,
    pub priority: String,
    pub rule: String,
}

/// A write the run would make in the tracker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedEffect {
    pub action: String,
    pub target: String,
}

/// What the port did with one admitted request.
#[derive(Debug, Clone, Default)]
pub struct Step {
    pub action_request: String,
    pub action: String,
    /// How the composition binds the action.
    pub binding: String,
    /// The Connectors operations called, in order.
    pub reads: Vec<String>,
    pub evidence: Option<EvidenceLine>,
    pub proposed: Option<ProposedEffect>,
    /// Why the port refused the request, when it did.
    pub refused: Option<String>,
    /// What else the trail shows: a classification, a review decision.
    pub detail: Option<String>,
}

/// The evidence one step submitted.
#[derive(Debug, Clone)]
pub struct EvidenceLine {
    pub kind: String,
    pub result: Option<String>,
    pub subject: String,
    pub revision: String,
}

/// The shared ledger.
pub type Shared = Arc<Mutex<Ledger>>;

/// The ledger, whatever a panicking holder left.
pub fn lock(ledger: &Shared) -> MutexGuard<'_, Ledger> {
    ledger.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Version-8 UUIDs from one counter, with the kind in the first group so two kinds never collide.
#[derive(Debug, Default)]
pub struct Ids(AtomicU64);

impl Ids {
    /// A new id of `kind`.
    pub fn next(&self, kind: u8) -> Uuid {
        let issued = self.0.fetch_add(1, Ordering::Relaxed) + 1;
        Uuid(format!("{kind:08x}-0000-8000-8000-{issued:012x}"))
    }
}

/// Id kinds.
pub mod kind {
    pub const RUN: u8 = 1;
    pub const ACTION_REQUEST: u8 = 2;
    pub const OBSERVATION: u8 = 3;
    pub const COMMISSION: u8 = 4;
    pub const AGENT_REVISION: u8 = 5;
    pub const EVIDENCE: u8 = 6;
    pub const READ_OBSERVATION: u8 = 7;
}

/// The effect port of one triage.
pub struct TriageEffects<'a> {
    pub governor: &'a CanonGovernor<MemoryCaseStore>,
    pub case: CaseId,
    pub ticket_id: u64,
    pub reads: &'a dyn ZendeskReads,
    pub composition: &'a Composition,
    pub ledger: Shared,
    pub ids: Arc<Ids>,
    /// The instant the run reads as now, Unix seconds.
    pub now: i64,
}

/// What a step decided, before the evidence is submitted.
struct Done {
    report: Value,
    evidence: Option<(String, Option<String>, String)>,
    provenance: Value,
    reads: Vec<String>,
    proposed: Option<ProposedEffect>,
    detail: Option<String>,
}

impl EffectPort for TriageEffects<'_> {
    fn performs(&self, action: &str) -> bool {
        self.composition.bindings.contains_key(action)
    }

    fn invoke(
        &self,
        _commission: &Commission<commission_state::Assigned>,
        request: &AdmittedRequest,
    ) -> Result<EffectOutcome, EffectError> {
        let data = request.data();
        let arguments = from_model(&data.arguments.0);
        let binding = self
            .composition
            .bindings
            .get(&data.action)
            .ok_or_else(|| EffectError::new(format!("`{}` has no binding", data.action)))?;
        let mut step = Step {
            action_request: data.action_request_id.0.0.clone(),
            action: data.action.clone(),
            binding: describe(binding),
            ..Step::default()
        };
        let done = match data.action.as_str() {
            "ticket.read" => self.read_ticket(),
            "requester.lookup" => self.lookup_requester(),
            "ticket.classify" => self.classify(&arguments),
            "classification.review" => self.review(),
            "ticket.route" => self.route(&arguments),
            "reply.draft" => Ok(Done {
                report: json!({"proposed": "reply.draft", "sent": false}),
                evidence: None,
                provenance: Value::Null,
                reads: Vec::new(),
                proposed: Some(ProposedEffect {
                    action: "reply.draft".to_owned(),
                    target: format!("ticket {}", self.ticket_id),
                }),
                detail: None,
            }),
            "escalation.request" => self.escalate(&arguments),
            other => Err(Refusal::Refused(format!("`{other}` is not performed here"))),
        };
        let outcome = match done {
            Ok(done) => {
                step.reads = done.reads;
                step.detail = done.detail;
                if let Some(proposed) = &done.proposed {
                    lock(&self.ledger).proposed.push(proposed.clone());
                }
                step.proposed = done.proposed;
                if let Some((kind, result, subject)) = done.evidence {
                    step.evidence = Some(self.submit(
                        &kind,
                        result,
                        &subject,
                        done.provenance,
                        &data.action,
                    )?);
                }
                EffectOutcome::Performed(EffectOutcomePerformed {
                    report: to_model(&done.report),
                })
            }
            Err(Refusal::Refused(reason)) => {
                step.refused = Some(reason.clone());
                EffectOutcome::Refused(EffectOutcomeRefused { reason })
            }
            Err(Refusal::Failed(reason)) => return Err(EffectError::new(reason)),
        };
        lock(&self.ledger).steps.push(step);
        Ok(outcome)
    }
}

/// Why a step did nothing: a refusal is an answer, a failure is not.
enum Refusal {
    Refused(String),
    Failed(String),
}

fn describe(binding: &Binding) -> String {
    match binding {
        Binding::ConnectorRead { operations } => {
            format!("connectors zendesk {}", operations.join(" + "))
        }
        Binding::Local { service } => format!("local {service}"),
        Binding::ProposedEffect => "recorded, never sent".to_owned(),
    }
}

fn read_failed(operation: &str, error: &ReadError) -> Refusal {
    Refusal::Failed(format!("{operation}: {error}"))
}

impl TriageEffects<'_> {
    fn operations(&self, action: &str) -> Vec<String> {
        match self.composition.bindings.get(action) {
            Some(Binding::ConnectorRead { operations }) => operations.clone(),
            _ => Vec::new(),
        }
    }

    fn timestamp(&self) -> Timestamp {
        Timestamp(crate::time::format(self.now))
    }

    fn current(&self, artifact: &str) -> Result<String, Refusal> {
        self.governor
            .revisions(&self.case)
            .map_err(|error| Refusal::Failed(format!("{error:?}")))?
            .get(artifact)
            .cloned()
            .ok_or_else(|| Refusal::Failed(format!("the case has no `{artifact}`")))
    }

    fn set_revision(&self, artifact: &str, revision: &str) -> Result<(), Refusal> {
        self.governor
            .update_revision(&self.case, artifact, revision)
            .map(|_| ())
            .map_err(|error| Refusal::Failed(error.to_string()))
    }

    /// `ticket.read`: the ticket and every page of its comments, through the bound operations.
    fn read_ticket(&self) -> Result<Done, Refusal> {
        let operations = self.operations("ticket.read");
        let (show, comments_op) = match operations.as_slice() {
            [show, comments] => (show.as_str(), comments.as_str()),
            _ => return Err(Refusal::Failed("ticket.read binds two reads".to_owned())),
        };
        let ticket = match self.reads.read(show, id_input("ticket_id", self.ticket_id)) {
            Ok(read) => read,
            Err(ReadError::NotFound) => {
                lock(&self.ledger).not_found = true;
                return Err(Refusal::Refused(format!(
                    "Zendesk has no ticket {}",
                    self.ticket_id
                )));
            }
            Err(error) => return Err(read_failed(show, &error)),
        };
        let mut reads = vec![show.to_owned()];
        let mut comments = Vec::new();
        let mut input = json!({"ticket_id": self.ticket_id, "page[size]": 100});
        for _ in 0..100 {
            let page = self
                .reads
                .read(comments_op, input.clone())
                .map_err(|error| read_failed(comments_op, &error))?;
            reads.push(comments_op.to_owned());
            comments.extend(
                page.body["comments"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
            if page.body["meta"]["has_more"] != Value::Bool(true) {
                break;
            }
            let Some(after) = page.body["meta"]["after_cursor"].as_str() else {
                break;
            };
            input["page[after]"] = Value::String(after.to_owned());
        }
        let body = &ticket.body["ticket"];
        let text = |name: &str| body[name].as_str().unwrap_or_default().to_owned();
        let requester = body["requester_id"].as_u64();
        let public_by = |by_requester: bool| {
            comments
                .iter()
                .filter(|comment| comment["public"] == Value::Bool(true))
                .filter(move |comment| (comment["author_id"].as_u64() == requester) == by_requester)
                .cloned()
                .collect::<Vec<_>>()
        };
        let requester_side: Vec<Value> = public_by(true)
            .iter()
            .map(|comment| json!([comment["id"], comment["body"]]))
            .collect();
        let revision = format!(
            "c-{}",
            &sha256_hex(
                json!([text("subject"), text("description"), requester_side])
                    .to_string()
                    .as_bytes()
            )[..12]
        );
        let priority = body["priority"].as_str().map(str::to_owned);
        let target = priority
            .as_deref()
            .and_then(|p| self.composition.first_reply_target_minutes.get(p))
            .or_else(|| self.composition.first_reply_target_minutes.get("normal"))
            .copied()
            .unwrap_or(480);
        let created = crate::time::parse(&text("created_at"));
        let replied = !public_by(false).is_empty();
        let breached = match created {
            Some(created) => !replied && self.now - created > target * 60,
            None => false,
        };
        let status = if breached {
            "breached"
        } else {
            "within_target"
        };
        self.set_revision("ticket", &revision)?;
        {
            let mut ledger = lock(&self.ledger);
            ledger.requester_id = requester;
            ledger.ticket = Some(TicketText {
                subject: text("subject"),
                description: text("description"),
                priority: priority.clone(),
            });
        }
        Ok(Done {
            report: json!({"revision": revision, "first_reply": status, "reads": reads}),
            evidence: Some((
                "ticket_snapshot".to_owned(),
                Some(status.to_owned()),
                "ticket".to_owned(),
            )),
            provenance: json!({
                "connector": self.composition.connector.provider,
                "operations": reads,
                "source_revision": ticket.source_revision,
                "first_reply_target_minutes": target,
                "replied": replied,
                "read_at": crate::time::format(self.now),
            }),
            reads,
            proposed: None,
            detail: Some(format!(
                "revision {revision}, priority {}, first reply {status}",
                priority.as_deref().unwrap_or("none")
            )),
        })
    }

    /// `requester.lookup`: the requester the ticket names, and their organization when they have
    /// one. Identified means active, not suspended and with an email address.
    fn lookup_requester(&self) -> Result<Done, Refusal> {
        let operations = self.operations("requester.lookup");
        let (user_op, organization_op) = match operations.as_slice() {
            [user, organization] => (user.as_str(), organization.as_str()),
            _ => {
                return Err(Refusal::Failed(
                    "requester.lookup binds two reads".to_owned(),
                ));
            }
        };
        let Some(requester) = lock(&self.ledger).requester_id else {
            return Err(Refusal::Refused(
                "the ticket read named no requester".to_owned(),
            ));
        };
        let mut reads = vec![user_op.to_owned()];
        let (revision, identified, organization) =
            match self.reads.read(user_op, id_input("user_id", requester)) {
                Ok(read) => {
                    let user = &read.body["user"];
                    let identified = user["active"] == Value::Bool(true)
                        && user["suspended"] != Value::Bool(true)
                        && user["email"]
                            .as_str()
                            .is_some_and(|email| email.contains('@'));
                    let revision = format!(
                        "u-{}",
                        &sha256_hex(
                            json!([user["id"], user["updated_at"]])
                                .to_string()
                                .as_bytes()
                        )[..12]
                    );
                    let organization = match user["organization_id"].as_u64() {
                        Some(organization) => {
                            reads.push(organization_op.to_owned());
                            match self
                                .reads
                                .read(organization_op, id_input("organization_id", organization))
                            {
                                Ok(_) => Some(organization),
                                Err(ReadError::NotFound) => None,
                                Err(error) => return Err(read_failed(organization_op, &error)),
                            }
                        }
                        None => None,
                    };
                    (revision, identified, organization)
                }
                Err(ReadError::NotFound) => (format!("u-missing-{requester}"), false, None),
                Err(error) => return Err(read_failed(user_op, &error)),
            };
        let result = if identified {
            "identified"
        } else {
            "unidentified"
        };
        self.set_revision("requester", &revision)?;
        Ok(Done {
            report: json!({"revision": revision, "result": result}),
            evidence: Some((
                "requester_profile".to_owned(),
                Some(result.to_owned()),
                "requester".to_owned(),
            )),
            provenance: json!({
                "connector": self.composition.connector.provider,
                "operations": reads,
                "organization": organization.is_some(),
            }),
            reads,
            proposed: None,
            detail: Some(format!(
                "requester {result}{}",
                if organization.is_some() {
                    ", organization found"
                } else {
                    ""
                }
            )),
        })
    }

    /// `ticket.classify`: the agent's proposal, checked against the deployment's lists.
    fn classify(&self, arguments: &Value) -> Result<Done, Refusal> {
        let field = |name: &str| arguments[name].as_str().unwrap_or_default().to_owned();
        let (category, priority, rule) = (field("category"), field("priority"), field("rule"));
        if !self.composition.categories.contains(&category) {
            return Err(Refusal::Refused(format!(
                "category `{category}` is not the deployment's"
            )));
        }
        if !self.composition.priorities.contains(&priority) {
            return Err(Refusal::Refused(format!(
                "priority `{priority}` is not the deployment's"
            )));
        }
        lock(&self.ledger).classification = Some(Classification {
            category: category.clone(),
            priority: priority.clone(),
            rule: rule.clone(),
        });
        Ok(Done {
            report: json!({"category": category, "priority": priority}),
            evidence: Some(("classification".to_owned(), None, "ticket".to_owned())),
            provenance: json!({"category": category, "priority": priority, "proposed_by": "agent"}),
            reads: Vec::new(),
            proposed: None,
            detail: Some(format!("proposed {category}/{priority}")),
        })
    }

    /// `classification.review`: the local reviewer, standing in for a person. It approves a
    /// proposal it can route and rejects `other`, which no queue takes.
    fn review(&self) -> Result<Done, Refusal> {
        let Some(proposal) = lock(&self.ledger).classification.clone() else {
            return Err(Refusal::Refused(
                "there is no proposal to review".to_owned(),
            ));
        };
        let decision = if self.composition.queues.contains_key(&proposal.category) {
            "approved"
        } else {
            "rejected"
        };
        lock(&self.ledger).review = Some(decision.to_owned());
        Ok(Done {
            report: json!({"decision": decision}),
            evidence: Some((
                "classification_review".to_owned(),
                Some(decision.to_owned()),
                "ticket".to_owned(),
            )),
            provenance: json!({"reviewer": "local reviewer", "category": proposal.category, "priority": proposal.priority}),
            reads: Vec::new(),
            proposed: None,
            detail: Some(format!(
                "{decision} {}/{} (local reviewer)",
                proposal.category, proposal.priority
            )),
        })
    }

    /// `ticket.route`: proposed only, to the queue the deployment gives the approved category.
    fn route(&self, arguments: &Value) -> Result<Done, Refusal> {
        let queue = arguments["queue"].as_str().unwrap_or_default().to_owned();
        let category = lock(&self.ledger)
            .classification
            .as_ref()
            .map(|c| c.category.clone())
            .unwrap_or_default();
        if self.composition.queues.get(&category) != Some(&queue) {
            return Err(Refusal::Refused(format!(
                "queue `{queue}` is not the deployment's queue for `{category}`"
            )));
        }
        Ok(Done {
            report: json!({"proposed": "ticket.route", "queue": queue, "sent": false}),
            evidence: Some(("routing_record".to_owned(), None, "ticket".to_owned())),
            provenance: json!({"queue": queue, "sent": false}),
            reads: Vec::new(),
            proposed: Some(ProposedEffect {
                action: "ticket.route".to_owned(),
                target: queue,
            }),
            detail: None,
        })
    }

    /// `escalation.request`: proposed only, to the deployment's escalation target.
    fn escalate(&self, arguments: &Value) -> Result<Done, Refusal> {
        let target = arguments["target"].as_str().unwrap_or_default().to_owned();
        if target != self.composition.escalation_target {
            return Err(Refusal::Refused(format!(
                "`{target}` is not the deployment's escalation target"
            )));
        }
        Ok(Done {
            report: json!({"proposed": "escalation.request", "target": target, "sent": false}),
            evidence: Some(("escalation_record".to_owned(), None, "ticket".to_owned())),
            provenance: json!({"target": target, "sent": false}),
            reads: Vec::new(),
            proposed: Some(ProposedEffect {
                action: "escalation.request".to_owned(),
                target,
            }),
            detail: None,
        })
    }

    /// Delivers the observation of a step and submits its evidence, citing it.
    fn submit(
        &self,
        kind: &str,
        result: Option<String>,
        subject: &str,
        provenance: Value,
        action: &str,
    ) -> Result<EvidenceLine, EffectError> {
        let failed = |error: String| EffectError::new(error);
        let revision = self.current(subject).map_err(|refusal| match refusal {
            Refusal::Refused(reason) | Refusal::Failed(reason) => failed(reason),
        })?;
        let observation = ObservationId(self.ids.next(kind::READ_OBSERVATION));
        self.governor
            .observe(Observation::new(ObservationData {
                observation_id: observation.clone(),
                source: PRODUCER.to_owned(),
                subject: format!("{subject}@{revision}"),
                observed_at: self.timestamp(),
                payload: to_model(&json!({"action": action, "kind": kind, "result": result})),
            }))
            .map_err(|error| failed(format!("{error:?}")))?;
        let evidence = EvidenceId(self.ids.next(kind::EVIDENCE));
        let held = self
            .governor
            .evidence(&self.case)
            .map_err(|error| failed(format!("{error:?}")))?
            .len();
        let mut record = json!({
            "format": "canon-evidence/1",
            "id": format!("{}-{}", kind.replace('_', "-"), held + 1),
            "kind": kind,
            "subject": subject,
            "subject_revision": revision,
        });
        if let Some(result) = &result {
            record["result"] = Value::String(result.clone());
        }
        let case_revision = self
            .governor
            .current_revision(&self.case)
            .map_err(|error| failed(format!("{error:?}")))?;
        submit_evidence(
            self.governor,
            PRODUCER,
            EvidenceData {
                evidence_id: evidence,
                case_id: self.case.clone(),
                kind: kind.to_owned(),
                subject_revision: case_revision,
                producer: PRODUCER.to_owned(),
                observation_ids: vec![observation],
                facts: to_model(&record),
                provenance: to_model(&provenance),
            },
        )
        .map_err(|error| failed(error.to_string()))?;
        Ok(EvidenceLine {
            kind: kind.to_owned(),
            result,
            subject: subject.to_owned(),
            revision,
        })
    }
}
