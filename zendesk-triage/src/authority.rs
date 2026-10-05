//! The authority provider: the composition's policy, plus what the operator approved for the run.
//!
//! Commission's runtime asks it at the moment of the call, for a capability the governor's
//! frontier names; the governor itself never grants one. `allow` is granted; `require_approval`
//! is granted only when the operator passed `--approve <capability>`, and otherwise answers
//! `ApprovalRequired`, which ends the run waiting for a person. A capability the composition names
//! no policy for is denied.

use std::collections::BTreeSet;
use std::sync::Mutex;

use loom_sdk::commission::model::responsibility::{
    AuthorityVerdict, AuthorityVerdictApprovalRequired, AuthorityVerdictDeny, CommissionData, Unit,
};
use loom_sdk::commission::ports::authority::{AuthorityProvider, AuthorityProviderError};

use crate::composition::{Composition, Policy};

/// One decision, as the trail shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub capability: String,
    pub verdict: String,
}

/// The policy authority of one run.
pub struct PolicyAuthority<'a> {
    composition: &'a Composition,
    approvals: BTreeSet<String>,
    decisions: Mutex<Vec<Decision>>,
}

impl<'a> PolicyAuthority<'a> {
    /// The composition's policy, with `approvals` granted by the operator.
    pub fn new(composition: &'a Composition, approvals: BTreeSet<String>) -> Self {
        Self {
            composition,
            approvals,
            decisions: Mutex::default(),
        }
    }

    /// Every decision made, in order.
    pub fn decisions(&self) -> Vec<Decision> {
        self.decisions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl AuthorityProvider for PolicyAuthority<'_> {
    fn decide(
        &self,
        _commission: &CommissionData,
        capability: &str,
    ) -> Result<AuthorityVerdict, AuthorityProviderError> {
        let (verdict, text) = match self.composition.policy_for(capability) {
            Some(Policy::Allow) => (AuthorityVerdict::Allow(Unit(true)), "allowed by policy"),
            Some(Policy::RequireApproval) if self.approvals.contains(capability) => (
                AuthorityVerdict::Allow(Unit(true)),
                "approved by the operator",
            ),
            Some(Policy::RequireApproval) => (
                AuthorityVerdict::ApprovalRequired(AuthorityVerdictApprovalRequired {
                    request: format!("approve `{capability}`: rerun with --approve {capability}"),
                }),
                "approval required",
            ),
            None => (
                AuthorityVerdict::Deny(AuthorityVerdictDeny {
                    reason: format!("the composition has no policy for `{capability}`"),
                }),
                "denied: no policy",
            ),
        };
        self.decisions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Decision {
                capability: capability.to_owned(),
                verdict: text.to_owned(),
            });
        Ok(verdict)
    }
}
