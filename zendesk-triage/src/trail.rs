//! The decision trail as text: every proposal, what the governor and the authority answered, what
//! the effect port did and the evidence it submitted, the claims after the run and the outcome.
//! It names ids, revisions and categories, never ticket text.

use std::fmt::Write as _;

use crate::triage::{Ended, Triage};

/// The trail of one triage.
pub fn render(triage: &Triage) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "ticket {}  case {}  protocol {} (ELS {}, sha256 {}…)",
        triage.ticket,
        triage.case,
        crate::protocol::PROTOCOL,
        &b10x_els::SOURCE_COMMIT[..7],
        crate::protocol::short_digest()
    );
    for (number, row) in triage.rows.iter().enumerate() {
        let _ = write!(
            out,
            "  {:>2}. {:<22} governor: {}",
            number + 1,
            row.action,
            row.governor
        );
        if let Some(authority) = &row.authority {
            let _ = write!(out, "; authority: {authority}");
        }
        out.push('\n');
        let Some(step) = &row.effect else {
            let _ = writeln!(out, "      not performed");
            continue;
        };
        let _ = write!(out, "      {}", step.binding);
        if let Some(detail) = &step.detail {
            let _ = write!(out, ": {detail}");
        }
        out.push('\n');
        if let Some(refused) = &step.refused {
            let _ = writeln!(out, "      refused by the effect port: {refused}");
        }
        if let Some(proposed) = &step.proposed {
            let _ = writeln!(
                out,
                "      proposed effect {} -> {} (not sent)",
                proposed.action, proposed.target
            );
        }
        if let Some(evidence) = &step.evidence {
            let result = evidence
                .result
                .as_ref()
                .map_or(String::new(), |result| format!(" = {result}"));
            let _ = writeln!(
                out,
                "      evidence {}{result} on {}@{}",
                evidence.kind, evidence.subject, evidence.revision
            );
        }
    }
    let claims: Vec<String> = triage
        .claims
        .iter()
        .map(|(claim, value)| format!("{claim}={value}"))
        .collect();
    let _ = writeln!(out, "  claims: {}", claims.join(" "));
    let obligations: Vec<String> = triage
        .obligations
        .iter()
        .map(|(obligation, open)| {
            format!("{obligation} {}", if *open { "open" } else { "discharged" })
        })
        .collect();
    let _ = writeln!(out, "  obligations: {}", obligations.join(", "));
    let _ = write!(out, "  outcome: {}", triage.ended);
    match &triage.ended {
        Ended::Triaged => {
            if let Some(classification) = &triage.classification {
                let _ = write!(
                    out,
                    " ({}/{}, route {})",
                    classification.category,
                    classification.priority,
                    triage.route().unwrap_or("none")
                );
            }
        }
        Ended::Escalated => {
            let _ = write!(out, " (to {})", triage.escalation().unwrap_or("none"));
        }
        _ => {}
    }
    out.push('\n');
    if !triage.proposed.is_empty() {
        let proposed: Vec<String> = triage
            .proposed
            .iter()
            .map(|effect| format!("{} -> {}", effect.action, effect.target))
            .collect();
        let _ = writeln!(
            out,
            "  proposed effects, none sent: {}",
            proposed.join(", ")
        );
    }
    out
}

/// One line per triage.
pub fn summary(triages: &[Triage]) -> String {
    let mut out = String::from("summary\n");
    for triage in triages {
        let _ = writeln!(
            out,
            "  ticket {:<6} {:<40} {} step(s)",
            triage.ticket,
            triage.ended.to_string(),
            triage.rows.len()
        );
    }
    out
}
