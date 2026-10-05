//! The agent Loom runs: a deterministic stand-in for a model, so a run needs no model key.
//!
//! [`RuleSelector`] is Loom's `ActionSelector`. It is handed only the catalogue Loom projected
//! from the governor's frontier, and picks the first action of [`STEP_ORDER`] the catalogue lists
//! that it has not picked before in this run. It cannot pick a blocked action: Loom projects none,
//! and refuses any id the catalogue does not list.
//!
//! [`RuleArguments`] is Loom's `ArgumentGenerator`. For `ticket.classify` it proposes a category
//! and priority from keyword rules over the ticket text read in this run; for `ticket.route` and
//! `escalation.request` it names the deployment's queue and target. What it writes is a proposal:
//! the effect port checks it, and the review decides on it.

use std::sync::Mutex;

use loom_sdk::commission::model::json::Value as Model;
use loom_sdk::loom::arguments::ArgumentContext;
use loom_sdk::loom::model::run::{CatalogueEntry, SelectionStrategy};
use loom_sdk::loom::selection::{Choice, SelectionContext, SelectorError};
use loom_sdk::{ActionSelector, ArgumentGenerator};
use serde_json::json;

use crate::composition::Composition;
use crate::effects::{Shared, TicketText, lock};
use crate::json::to_model;

/// The order the rule selector prefers: read first, escalate a breach at once, then know the
/// requester, classify, review and route.
pub const STEP_ORDER: [&str; 6] = [
    "ticket.read",
    "escalation.request",
    "requester.lookup",
    "ticket.classify",
    "classification.review",
    "ticket.route",
];

/// Keyword rules: the first category with a keyword in the subject or description wins.
pub const RULES: [(&str, &[&str]); 3] = [
    (
        "billing",
        &[
            "invoice", "charged", "charge", "refund", "payment", "billing",
        ],
    ),
    (
        "technical",
        &["error", "fails", "outage", "crash", "bug", "not working"],
    ),
    ("account", &["log in", "login", "password", "sign in"]),
];

/// The classification the rules propose for `ticket`: category, priority and the rule that fired.
pub fn classify(ticket: &TicketText, priorities: &[String]) -> (String, String, String) {
    let text = format!("{} {}", ticket.subject, ticket.description).to_lowercase();
    let (category, rule) = RULES
        .iter()
        .find_map(|(category, keywords)| {
            keywords
                .iter()
                .find(|keyword| text.contains(*keyword))
                .map(|keyword| ((*category).to_owned(), format!("keyword `{keyword}`")))
        })
        .unwrap_or_else(|| ("other".to_owned(), "no rule matched".to_owned()));
    let priority = ticket
        .priority
        .clone()
        .filter(|priority| priorities.contains(priority))
        .unwrap_or_else(|| "normal".to_owned());
    (category, priority, rule)
}

/// The rule selector of one run.
pub struct RuleSelector {
    ledger: Shared,
    picked: Mutex<Vec<String>>,
}

impl RuleSelector {
    /// A selector that reads the run's ledger.
    pub fn new(ledger: Shared) -> Self {
        Self {
            ledger,
            picked: Mutex::default(),
        }
    }
}

impl ActionSelector for RuleSelector {
    fn select(
        &self,
        _context: &SelectionContext,
        candidates: &[CatalogueEntry],
    ) -> Result<Choice, SelectorError> {
        if lock(&self.ledger).not_found {
            return Err(SelectorError::Unavailable(
                "the ticket was not found".to_owned(),
            ));
        }
        let mut picked = self
            .picked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = STEP_ORDER.iter().find(|action| {
            !picked.iter().any(|done| done == *action)
                && candidates.iter().any(|entry| entry.action == **action)
        });
        match next {
            Some(action) => {
                picked.push((*action).to_owned());
                Ok(Choice {
                    action: (*action).to_owned(),
                    confidence: None,
                })
            }
            None => Err(SelectorError::NothingAdmissible),
        }
    }

    fn strategy(&self) -> SelectionStrategy {
        SelectionStrategy::Rule
    }
}

/// The rule argument generator of one run.
pub struct RuleArguments<'a> {
    ledger: Shared,
    composition: &'a Composition,
}

impl<'a> RuleArguments<'a> {
    /// A generator that reads the run's ledger and the deployment's lists.
    pub fn new(ledger: Shared, composition: &'a Composition) -> Self {
        Self {
            ledger,
            composition,
        }
    }
}

impl ArgumentGenerator for RuleArguments<'_> {
    fn generate(
        &self,
        _context: &ArgumentContext,
        entry: &CatalogueEntry,
    ) -> Result<Model, String> {
        let ledger = lock(&self.ledger);
        let arguments = match entry.action.as_str() {
            "ticket.classify" => {
                let ticket = ledger
                    .ticket
                    .as_ref()
                    .ok_or("the ticket has not been read in this run")?;
                let (category, priority, rule) = classify(ticket, &self.composition.priorities);
                json!({"category": category, "priority": priority, "rule": rule})
            }
            "ticket.route" => {
                let category = ledger
                    .classification
                    .as_ref()
                    .map(|c| c.category.clone())
                    .unwrap_or_default();
                let queue = self
                    .composition
                    .queues
                    .get(&category)
                    .cloned()
                    .unwrap_or_default();
                json!({"queue": queue})
            }
            "escalation.request" => json!({"target": self.composition.escalation_target}),
            _ => json!({}),
        };
        Ok(to_model(&arguments))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(subject: &str, description: &str, priority: Option<&str>) -> TicketText {
        TicketText {
            subject: subject.to_owned(),
            description: description.to_owned(),
            priority: priority.map(str::to_owned),
        }
    }

    #[test]
    fn rules_pick_the_first_matching_category() {
        let priorities = ["low", "normal", "high", "urgent"].map(str::to_owned);
        let classify = |t: &TicketText| classify(t, &priorities);
        assert_eq!(
            classify(&ticket("Invoice charged twice", "", Some("normal"))).0,
            "billing"
        );
        assert_eq!(
            classify(&ticket("Checkout returns errors", "", Some("urgent"))),
            (
                "technical".to_owned(),
                "urgent".to_owned(),
                "keyword `error`".to_owned()
            )
        );
        assert_eq!(
            classify(&ticket("A question", "Hello", Some("bogus"))),
            (
                "other".to_owned(),
                "normal".to_owned(),
                "no rule matched".to_owned()
            )
        );
    }
}
