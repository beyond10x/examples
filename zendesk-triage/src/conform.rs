//! Runs the ESS suite authored under `ess/` against the triage, as a runner outside ESS.
//!
//! `ess verify conform author` compiles the scenarios into a suite; this runner executes each
//! scenario's `TriageTicket` against the fixture Zendesk, compares what the run ended with to each
//! expectation, and writes `ess-conformance-results/1`, which `ess verify conform report` turns
//! into a report. It reads the suite's expectations to judge them and never to decide an answer.
//!
//! Step vocabulary it executes: `configure_external_outcome`, `execute_command`, `expect_outcome`,
//! `expect_error`, `expect_event` and `expect_no_event`. Any other step makes its scenario
//! `unsupported`. The fixture decides which external outcome a ticket reaches; a forced outcome is
//! therefore an arrangement the fixture must already meet, and is checked against the actual one.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::composition::Composition;
use crate::triage::{Ended, Options, Triage};
use crate::zendesk::ZendeskReads;

/// The command every scenario drives.
pub const COMMAND: &str = "examples.support_triage.TriageTicket";

/// What one `TriageTicket` answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub outcome: Option<String>,
    pub error: Option<String>,
    /// (event name, payload).
    pub events: Vec<(String, Value)>,
}

/// The declared answer of one triage.
pub fn answer(triage: &Triage) -> Answer {
    let id = triage_id(&triage.ticket);
    let event = |name: &str, mut payload: Map<String, Value>| {
        payload.insert("triage_id".to_owned(), Value::String(id.clone()));
        payload.insert("ticket".to_owned(), Value::String(triage.ticket.clone()));
        (
            format!("examples.support_triage.{name}"),
            Value::Object(payload),
        )
    };
    let fields = |value: Value| match value {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let (outcome, error, events) = match &triage.ended {
        Ended::Triaged => {
            let classification = triage.classification.clone();
            (
                Some("triaged"),
                None,
                vec![event(
                    "TicketTriaged",
                    fields(json!({
                        "category": classification.as_ref().map(|c| c.category.clone()),
                        "priority": classification.as_ref().map(|c| c.priority.clone()),
                        "route": triage.route(),
                    })),
                )],
            )
        }
        Ended::Escalated => (
            Some("escalated"),
            None,
            vec![event(
                "TicketEscalated",
                fields(json!({"escalation": triage.escalation()})),
            )],
        ),
        Ended::NeedsHuman { reason } => (
            Some("needs-human"),
            None,
            vec![event(
                "TicketHandedToHuman",
                fields(json!({"reason": reason})),
            )],
        ),
        Ended::AwaitingApproval { actions } => (
            Some("awaiting-approval"),
            None,
            vec![event(
                "ApprovalAwaited",
                fields(json!({"actions": actions})),
            )],
        ),
        Ended::UnknownTicket => (
            Some("unknown-ticket"),
            Some("examples.support_triage.TicketNotFound".to_owned()),
            Vec::new(),
        ),
        Ended::Stopped { .. } => (None, None, Vec::new()),
    };
    Answer {
        outcome: outcome.map(str::to_owned),
        error,
        events,
    }
}

/// A version-8 UUID naming the triage of `ticket`.
pub fn triage_id(ticket: &str) -> String {
    let hex = crate::json::sha256_hex(format!("triage\n{ticket}").as_bytes());
    format!(
        "{}-{}-8{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

/// One scenario's terminal status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub scenario: String,
    pub status: &'static str,
    pub message: Option<String>,
}

/// Runs every scenario of `suite` with `reads`, under `composition`.
///
/// # Errors
/// When the suite is not an `ess-conformance` suite of this system.
pub fn run(
    suite: &Value,
    reads: &dyn ZendeskReads,
    composition: &Composition,
) -> Result<Vec<Verdict>, String> {
    if suite["provenance"]["system"] != "examples" {
        return Err("the suite is not of the `examples` system".to_owned());
    }
    let scenarios = suite["scenarios"]
        .as_object()
        .ok_or("the suite has no scenarios")?;
    Ok(scenarios
        .iter()
        .map(|(id, scenario)| {
            let (status, message) = match scenario_status(scenario, reads, composition) {
                Ok(()) => ("passed", None),
                Err(Status::Failed(message)) => ("failed", Some(message)),
                Err(Status::Unsupported(message)) => ("unsupported", Some(message)),
                Err(Status::Error(message)) => ("error", Some(message)),
            };
            Verdict {
                scenario: id.clone(),
                status,
                message,
            }
        })
        .collect())
}

enum Status {
    Failed(String),
    Unsupported(String),
    Error(String),
}

fn scenario_status(
    scenario: &Value,
    reads: &dyn ZendeskReads,
    composition: &Composition,
) -> Result<(), Status> {
    let steps = scenario["steps"]
        .as_array()
        .ok_or_else(|| Status::Error("the scenario has no steps".to_owned()))?;
    let mut forced: Option<String> = None;
    let mut last: Option<Answer> = None;
    let answered = |last: &Option<Answer>| {
        last.clone()
            .ok_or_else(|| Status::Error("an expectation precedes every command".to_owned()))
    };
    for step in steps {
        match step["step"].as_str().unwrap_or_default() {
            "configure_external_outcome" => {
                forced = step["force"]["outcome"].as_str().map(str::to_owned);
            }
            "execute_command" => {
                if step["command"] != COMMAND {
                    return Err(Status::Unsupported(format!("command {}", step["command"])));
                }
                let literal = |name: &str| {
                    let input = &step["input"][name];
                    if input["kind"] == "literal" {
                        Ok(input["value"].clone())
                    } else {
                        Err(Status::Unsupported(format!(
                            "input `{name}` is not a literal"
                        )))
                    }
                };
                let ticket = literal("ticket")?;
                let ticket = ticket
                    .as_str()
                    .ok_or_else(|| Status::Error("ticket is not text".to_owned()))?;
                let approvals: BTreeSet<String> = literal("approvals")?
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect();
                let options = Options {
                    approvals,
                    now: crate::triage::FIXTURE_NOW,
                };
                let triage = crate::triage::run(reads, composition, ticket, &options)
                    .map_err(Status::Error)?;
                let got = answer(&triage);
                if let Some(forced) = &forced
                    && got.outcome.as_ref() != Some(forced)
                {
                    return Err(Status::Failed(format!(
                        "the scenario arranges outcome `{forced}`; the fixture ticket {ticket} reached {:?}",
                        got.outcome
                    )));
                }
                last = Some(got);
            }
            "expect_outcome" => {
                let want = step["outcome"]["outcome"].as_str();
                let got = answered(&last)?;
                if got.outcome.as_deref() != want {
                    return Err(Status::Failed(format!(
                        "outcome {:?}, expected {want:?}",
                        got.outcome
                    )));
                }
            }
            "expect_error" => {
                let want = step["error"].as_str();
                let got = answered(&last)?;
                if got.error.as_deref() != want {
                    return Err(Status::Failed(format!(
                        "error {:?}, expected {want:?}",
                        got.error
                    )));
                }
            }
            "expect_event" => {
                let name = step["event"].as_str().unwrap_or_default();
                let got = answered(&last)?;
                let (_, payload) = got
                    .events
                    .iter()
                    .find(|(event, _)| event == name)
                    .ok_or_else(|| Status::Failed(format!("no event {name}")))?;
                for (field, want) in step["payload"].as_object().into_iter().flatten() {
                    if &payload[field] != want {
                        return Err(Status::Failed(format!(
                            "{name}.{field} is {}, expected {want}",
                            payload[field]
                        )));
                    }
                }
                for (field, shape) in step["shape"].as_object().into_iter().flatten() {
                    if !fits(&payload[field], shape) {
                        return Err(Status::Failed(format!(
                            "{name}.{field} = {} does not fit {shape}",
                            payload[field]
                        )));
                    }
                }
            }
            "expect_no_event" => {
                let name = step["event"].as_str().unwrap_or_default();
                if answered(&last)?
                    .events
                    .iter()
                    .any(|(event, _)| event == name)
                {
                    return Err(Status::Failed(format!("unexpected event {name}")));
                }
            }
            other => return Err(Status::Unsupported(format!("step `{other}`"))),
        }
    }
    Ok(())
}

/// Whether `value` fits the suite's `shape` of a payload field.
fn fits(value: &Value, shape: &Value) -> bool {
    match (shape["holds"].as_str(), shape["kind"].as_str()) {
        (Some("primitive"), Some("string")) => value.is_string(),
        (Some("primitive"), Some("uuid")) => value.as_str().is_some_and(|text| {
            text.len() == 36
                && text.char_indices().all(|(at, c)| match at {
                    8 | 13 | 18 | 23 => c == '-',
                    _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
                })
        }),
        (Some("list"), _) => value.is_array(),
        _ => !value.is_null(),
    }
}

/// The `ess-conformance-results/1` document of `verdicts`.
pub fn results(verdicts: &[Verdict], completed_at: u64) -> Value {
    json!({
        "format": "ess-conformance-results/1",
        "completed_at": completed_at,
        "results": verdicts.iter().map(|verdict| {
            let mut result = json!({"scenario_id": verdict.scenario, "status": verdict.status});
            if let Some(message) = &verdict.message {
                result["message"] = Value::String(message.clone());
            }
            result
        }).collect::<Vec<_>>(),
    })
}

/// Reads a suite file.
///
/// # Errors
/// When it cannot be read or is not JSON.
pub fn read_suite(path: &Path) -> Result<Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}
