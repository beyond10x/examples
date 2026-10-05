//! The triage of each fixture ticket, end to end: Connectors engine over the fixture Zendesk, Loom
//! as executor, the governor evaluating support.triage/1 with Canon.

use std::collections::BTreeSet;

use zendesk_triage::composition::Composition;
use zendesk_triage::triage::{Ended, FIXTURE_NOW, Options, Triage, run};
use zendesk_triage::zendesk::{fixture_reads, fixture_root};

fn triage(ticket: &str, approvals: &[&str]) -> Triage {
    let composition = Composition::shipped().expect("the shipped composition parses");
    let reads = fixture_reads(&fixture_root(), &composition.connector.instance)
        .expect("the connectors engine loads the vendored bundle");
    let options = Options {
        approvals: approvals
            .iter()
            .map(|a| (*a).to_owned())
            .collect::<BTreeSet<_>>(),
        now: FIXTURE_NOW,
    };
    run(&reads, &composition, ticket, &options).expect("the triage runs")
}

fn actions(triage: &Triage) -> Vec<&str> {
    triage.rows.iter().map(|row| row.action.as_str()).collect()
}

#[test]
fn a_ticket_within_target_is_classified_reviewed_and_routed() {
    let triage = triage("1001", &["ticket.route"]);
    assert_eq!(triage.ended, Ended::Triaged);
    assert_eq!(
        actions(&triage),
        [
            "ticket.read",
            "requester.lookup",
            "ticket.classify",
            "classification.review",
            "ticket.route"
        ]
    );
    assert_eq!(triage.claim("ticket.triaged"), Some("true"));
    assert_eq!(triage.claim("sla.breached"), Some("false"));
    let classification = triage.classification.clone().unwrap();
    assert_eq!(
        (
            classification.category.as_str(),
            classification.priority.as_str()
        ),
        ("billing", "normal")
    );
    assert_eq!(triage.route(), Some("queue-billing"));
    assert_eq!(
        triage.obligations,
        [("escalate_sla_breach".to_owned(), false)]
    );
    // Routing passed through the governor as approval-required, then the operator's approval.
    let route = triage.rows.last().unwrap();
    assert_eq!(route.governor, "approval-required (ticket.route)");
    assert_eq!(route.authority.as_deref(), Some("approved by the operator"));
}

#[test]
fn without_the_routing_approval_the_run_stops_and_proposes_nothing() {
    let triage = triage("1001", &[]);
    assert_eq!(
        triage.ended,
        Ended::AwaitingApproval {
            actions: vec!["ticket.route".to_owned()]
        }
    );
    assert!(triage.proposed.is_empty());
    assert_eq!(triage.claim("ticket.classified"), Some("true"));
    assert_eq!(triage.claim("ticket.routed"), Some("unknown"));
    let route = triage.rows.last().unwrap();
    assert!(
        route.effect.is_none(),
        "the port was never handed the route"
    );
}

#[test]
fn a_breached_ticket_is_escalated_before_anything_else() {
    let triage = triage("1002", &[]);
    assert_eq!(triage.ended, Ended::Escalated);
    assert_eq!(actions(&triage), ["ticket.read", "escalation.request"]);
    assert_eq!(triage.claim("sla.breached"), Some("true"));
    assert_eq!(triage.escalation(), Some("support-oncall"));
    assert_eq!(
        triage.obligations,
        [("escalate_sla_breach".to_owned(), false)]
    );
}

#[test]
fn a_suspended_requester_sends_the_ticket_to_a_person() {
    let triage = triage("1003", &["ticket.route"]);
    assert_eq!(
        triage.ended,
        Ended::NeedsHuman {
            reason: "requester_unidentified".to_owned()
        }
    );
    assert_eq!(actions(&triage), ["ticket.read", "requester.lookup"]);
    assert_eq!(triage.claim("requester.known"), Some("false"));
    assert!(triage.proposed.is_empty());
}

#[test]
fn a_rejected_classification_sends_the_ticket_to_a_person() {
    let triage = triage("1004", &["ticket.route"]);
    assert_eq!(
        triage.ended,
        Ended::NeedsHuman {
            reason: "classification_rejected".to_owned()
        }
    );
    assert_eq!(triage.review.as_deref(), Some("rejected"));
    assert_eq!(triage.claim("classification.reviewed"), Some("false"));
    assert!(triage.route().is_none());
}

#[test]
fn an_unknown_ticket_is_refused_and_nothing_follows() {
    let triage = triage("9999", &["ticket.route"]);
    assert_eq!(triage.ended, Ended::UnknownTicket);
    assert_eq!(actions(&triage), ["ticket.read"]);
    assert!(triage.rows[0].effect.as_ref().unwrap().refused.is_some());
}

#[test]
fn the_trail_carries_no_ticket_text_or_requester_details() {
    for ticket in ["1001", "1002", "1003", "1004"] {
        let rendered = zendesk_triage::trail::render(&triage(ticket, &["ticket.route"]));
        for text in [
            "Invoice",
            "checkout",
            "password",
            "a question",
            "example.com",
            "Requester A",
            "Requester C",
            "Organization A",
        ] {
            assert!(
                !rendered.to_lowercase().contains(&text.to_lowercase()),
                "ticket {ticket}: the trail shows `{text}`:\n{rendered}"
            );
        }
    }
}

#[test]
fn a_ticket_id_that_is_not_a_number_is_refused_before_any_read() {
    let composition = Composition::shipped().unwrap();
    let reads = fixture_reads(&fixture_root(), "zendesk-support").unwrap();
    let options = Options {
        approvals: BTreeSet::new(),
        now: FIXTURE_NOW,
    };
    assert!(run(&reads, &composition, "../1001", &options).is_err());
    assert!(reads.http().requests().is_empty());
}

#[test]
fn the_reads_go_through_the_connectors_engine_with_the_bound_operations() {
    let composition = Composition::shipped().unwrap();
    let reads = fixture_reads(&fixture_root(), "zendesk-support").unwrap();
    let options = Options {
        approvals: ["ticket.route".to_owned()].into(),
        now: FIXTURE_NOW,
    };
    run(&reads, &composition, "1001", &options).unwrap();
    assert_eq!(
        reads.http().requests(),
        [
            "GET /api/v2/tickets/1001?",
            "GET /api/v2/tickets/1001/comments?page[size]=100",
            "GET /api/v2/users/601?",
            "GET /api/v2/organizations/501?",
        ]
    );
}
