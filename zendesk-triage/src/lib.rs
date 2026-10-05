#![forbid(unsafe_code)]

//! Triage of one support ticket, run on one machine by the governed-autonomy stack.
//!
//! - The rules are ELS's `support.triage/1` ([`protocol`]), evaluated by Loom's governor with
//!   Canon. The governor decides; it never acts.
//! - Loom is the executor: a selector picks one action from the frontier's catalogue and an
//!   argument generator writes its arguments ([`agent`]). In this demo both are deterministic
//!   rules, so a run needs no model.
//! - Commission's runtime (in Loom) revalidates every proposal against the current frontier and
//!   asks the authority provider ([`authority`]) for any capability the frontier names.
//! - The effect port ([`effects`]) performs what the runtime admits, through the binding in
//!   `composition.yaml` ([`composition`]): reads go to Zendesk through Connectors
//!   ([`zendesk`]); route, reply and escalation are recorded as proposed effects and never sent.
//!   Trusted code in the effect port, not the agent, turns what was read into evidence.
//! - [`triage`] wires one run per ticket and returns its decision trail; [`conform`] holds the run
//!   to the ESS scenarios under `ess/`; [`pins`] checks the vendored protocol and connectors files
//!   against the commits they were copied from.

pub mod agent;
pub mod authority;
pub mod composition;
pub mod conform;
pub mod effects;
pub mod json;
pub mod pins;
pub mod protocol;
pub mod time;
pub mod trail;
pub mod triage;
pub mod zendesk;
