//! The deployment data the protocol leaves open: which tool serves each action, which capability
//! needs a person, and the lists a classification and a route choose from.
//!
//! It stands in for a `composition/1` document (Atlas ADR 0087), which has no format yet. The
//! protocol names actions only; the binding to Connectors operations lives here (ADR 0083).

use std::collections::BTreeMap;

use b10x_canon::ir::Ir;
use serde::Deserialize;

/// The composition this crate ships, `composition.yaml`.
pub const SHIPPED: &str = include_str!("../composition.yaml");

/// The format this reader accepts.
pub const FORMAT: &str = "examples-triage-composition/1";

/// A composition.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    pub format: String,
    /// The registry name of the protocol, `<name>@<major>`.
    pub protocol: String,
    pub connector: Connector,
    /// One binding per protocol action.
    pub bindings: BTreeMap<String, Binding>,
    /// One policy per capability the protocol requires.
    pub policy: BTreeMap<String, Policy>,
    pub categories: Vec<String>,
    pub priorities: Vec<String>,
    /// The queue a ticket of each category is routed to.
    pub queues: BTreeMap<String, String>,
    pub escalation_target: String,
    /// The first-reply target of each priority, in minutes.
    pub first_reply_target_minutes: BTreeMap<String, i64>,
}

/// The Connectors provider and instance the reads go to.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connector {
    pub provider: String,
    pub instance: String,
}

/// What serves one protocol action.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    /// Read operations of the composition's connector, in the order they are called.
    ConnectorRead { operations: Vec<String> },
    /// A service in this process that changes nothing outside the case.
    Local { service: String },
    /// A write that is recorded as proposed and never sent.
    ProposedEffect,
}

/// Whether a capability is granted without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// Granted.
    Allow,
    /// Granted only when the operator approved it for the run.
    RequireApproval,
}

impl Composition {
    /// The shipped composition.
    ///
    /// # Errors
    /// When it does not parse.
    pub fn shipped() -> Result<Self, String> {
        Self::parse(SHIPPED)
    }

    /// A composition from its YAML text.
    ///
    /// # Errors
    /// When the text is not a composition of [`FORMAT`].
    pub fn parse(text: &str) -> Result<Self, String> {
        let composition: Self =
            serde_yaml_ng::from_str(text).map_err(|error| format!("composition: {error}"))?;
        if composition.format != FORMAT {
            return Err(format!(
                "composition: format `{}` is not `{FORMAT}`",
                composition.format
            ));
        }
        Ok(composition)
    }

    /// Every problem with this composition against the compiled protocol and the read operations
    /// the connector exposes; empty when there is none.
    pub fn problems(&self, protocol: &Ir, reads: &[String]) -> Vec<String> {
        let mut problems = Vec::new();
        if self.protocol != crate::protocol::PROTOCOL {
            problems.push(format!(
                "the composition is for `{}`, not `{}`",
                self.protocol,
                crate::protocol::PROTOCOL
            ));
        }
        for (action, declared) in &protocol.actions {
            let action = action.as_str();
            let effect = declared.effect.as_ref().map(|effect| effect.as_str());
            match (self.bindings.get(action), effect) {
                (None, _) => problems.push(format!("action `{action}` has no binding")),
                (Some(Binding::ConnectorRead { operations }), Some("read")) => {
                    if operations.is_empty() {
                        problems.push(format!("action `{action}` binds no operation"));
                    }
                    for operation in operations {
                        if !reads.contains(operation) {
                            problems.push(format!(
                                "action `{action}` binds `{operation}`, which the connector does not expose as a read"
                            ));
                        }
                    }
                }
                (Some(Binding::Local { .. }), Some("none")) => {}
                (Some(Binding::ProposedEffect), Some("write")) => {}
                (Some(binding), effect) => problems.push(format!(
                    "action `{action}` has effect `{}` and cannot be bound as {binding:?}",
                    effect.unwrap_or("unspecified")
                )),
            }
            for capability in &declared.requires {
                let capability = capability.as_str();
                if !self.policy.contains_key(capability) {
                    problems.push(format!("capability `{capability}` has no policy"));
                }
            }
        }
        for action in self.bindings.keys() {
            if !protocol
                .actions
                .keys()
                .any(|declared| declared.as_str() == action)
            {
                problems.push(format!(
                    "binding `{action}` names no action of the protocol"
                ));
            }
        }
        for capability in self.policy.keys() {
            if !protocol
                .actions
                .values()
                .any(|declared| declared.requires.iter().any(|c| c.as_str() == capability))
            {
                problems.push(format!(
                    "policy `{capability}` names no capability of the protocol"
                ));
            }
        }
        for category in self.queues.keys() {
            if !self.categories.contains(category) {
                problems.push(format!("queue for unknown category `{category}`"));
            }
        }
        for priority in &self.priorities {
            if !self.first_reply_target_minutes.contains_key(priority) {
                problems.push(format!("priority `{priority}` has no first-reply target"));
            }
        }
        problems
    }

    /// The policy for `capability`; `None` when the composition names none.
    pub fn policy_for(&self, capability: &str) -> Option<Policy> {
        self.policy.get(capability).copied()
    }
}
