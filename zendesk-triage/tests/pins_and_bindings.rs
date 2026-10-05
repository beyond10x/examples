//! The pins hold, and the composition binds every protocol action to something that fits it.

use zendesk_triage::composition::Composition;
use zendesk_triage::{pins, protocol, zendesk};

fn reads() -> Vec<String> {
    zendesk::engine()
        .unwrap()
        .declarations(&[connectors_catalog_provider::Effect::Read])
        .into_iter()
        .map(|operation| operation.id)
        .collect()
}

#[test]
fn the_vendored_protocol_is_the_one_els_embeds_at_the_pinned_commit() {
    let held = pins::check_protocol().expect("the protocol pin holds");
    assert!(
        held.iter()
            .any(|line| line.contains("59fd19f639a6427533cd63c5ca53eaaf040d70c7"))
    );
}

#[test]
fn the_vendored_connectors_files_match_their_digests() {
    pins::check_connectors(None).expect("the connectors pin holds");
}

#[test]
fn connectors_exposes_exactly_the_seven_zendesk_reads() {
    let mut reads = reads();
    reads.sort();
    assert_eq!(
        reads,
        [
            "organization.show",
            "organizations.incremental",
            "ticket.comments",
            "ticket.show",
            "tickets.incremental",
            "user.show",
            "users.incremental"
        ]
    );
}

#[test]
fn the_shipped_composition_fits_the_protocol_and_the_connector() {
    let composition = Composition::shipped().unwrap();
    let problems = composition.problems(&protocol::compiled().unwrap(), &reads());
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn a_write_bound_to_a_connector_read_or_an_unknown_operation_is_refused() {
    let shipped = zendesk_triage::composition::SHIPPED;
    let write_as_read = shipped.replace(
        "  ticket.route:\n    kind: proposed_effect",
        "  ticket.route:\n    kind: connector_read\n    operations: [ticket.show]",
    );
    assert_ne!(write_as_read, shipped);
    let composition = Composition::parse(&write_as_read).unwrap();
    let problems = composition.problems(&protocol::compiled().unwrap(), &reads());
    assert!(
        problems.iter().any(|p| p.contains("ticket.route")),
        "{problems:#?}"
    );

    let unknown = shipped.replace("[user.show, organization.show]", "[user.show, user.update]");
    let composition = Composition::parse(&unknown).unwrap();
    let problems = composition.problems(&protocol::compiled().unwrap(), &reads());
    assert!(
        problems.iter().any(|p| p.contains("user.update")),
        "{problems:#?}"
    );
}

#[test]
fn every_protocol_action_name_is_tool_agnostic() {
    let ir = protocol::compiled().unwrap();
    for action in ir.actions.keys() {
        assert!(!action.as_str().contains("zendesk"), "{}", action.as_str());
    }
}
