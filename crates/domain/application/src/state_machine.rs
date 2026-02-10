// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Configurable state machine for application status transitions.
//!
//! The [`StateMachine`] validates that a requested transition from one
//! [`ApplicationStatus`] to another is legal according to a set of
//! [`TransitionRule`]s.  Default rules encode the standard lifecycle:
//!
//! ```text
//! Draft -> Submitted, Withdrawn
//! Submitted -> UnderReview, Rejected, Withdrawn
//! UnderReview -> Interview, Rejected, Withdrawn
//! Interview -> Offered, Rejected, Withdrawn
//! Offered -> Accepted, Rejected, Withdrawn
//! Rejected -> (terminal)
//! Accepted -> (terminal)
//! Withdrawn -> (terminal)
//! ```

use std::collections::HashMap;

use crate::{
    error::{ApplicationError, InvalidTransitionSnafu},
    types::ApplicationStatus,
};

// ---------------------------------------------------------------------------
// TransitionRule
// ---------------------------------------------------------------------------

/// A single rule mapping a source status to the set of statuses it may
/// transition to.
#[derive(Debug, Clone)]
pub struct TransitionRule {
    /// The status this rule applies to.
    pub from:       ApplicationStatus,
    /// The statuses that are reachable from `from`.
    pub allowed_to: Vec<ApplicationStatus>,
}

// ---------------------------------------------------------------------------
// StateMachine
// ---------------------------------------------------------------------------

/// A configurable engine that validates application status transitions.
///
/// Create one with [`StateMachine::default()`] for the standard
/// lifecycle rules, or build a custom one with
/// [`StateMachine::with_rules`].
#[derive(Debug, Clone)]
pub struct StateMachine {
    rules: HashMap<ApplicationStatus, Vec<ApplicationStatus>>,
}

impl StateMachine {
    /// Create a state machine from a custom set of transition rules.
    #[must_use]
    pub fn with_rules(rules: Vec<TransitionRule>) -> Self {
        let map = rules.into_iter().map(|r| (r.from, r.allowed_to)).collect();
        Self { rules: map }
    }

    /// Validate that transitioning from `from` to `to` is allowed.
    ///
    /// Returns `Ok(())` if the transition is legal, or
    /// [`ApplicationError::InvalidTransition`] otherwise.
    pub fn validate_transition(
        &self,
        from: ApplicationStatus,
        to: ApplicationStatus,
    ) -> Result<(), ApplicationError> {
        let allowed = self
            .rules
            .get(&from)
            .map_or(false, |targets| targets.contains(&to));

        if allowed {
            Ok(())
        } else {
            InvalidTransitionSnafu { from, to }.fail()
        }
    }

    /// Return the set of statuses reachable from the given status.
    #[must_use]
    pub fn allowed_transitions(&self, from: ApplicationStatus) -> Vec<ApplicationStatus> {
        self.rules.get(&from).cloned().unwrap_or_default()
    }

    /// Return `true` if the given status is terminal (has no outgoing
    /// transitions).
    #[must_use]
    pub fn is_terminal(&self, status: ApplicationStatus) -> bool {
        self.rules.get(&status).map_or(true, Vec::is_empty)
    }
}

impl Default for StateMachine {
    /// Build the standard lifecycle state machine.
    fn default() -> Self {
        use ApplicationStatus::{
            Accepted, Draft, Interview, Offered, Rejected, Submitted, UnderReview, Withdrawn,
        };

        Self::with_rules(vec![
            TransitionRule {
                from:       Draft,
                allowed_to: vec![Submitted, Withdrawn],
            },
            TransitionRule {
                from:       Submitted,
                allowed_to: vec![UnderReview, Rejected, Withdrawn],
            },
            TransitionRule {
                from:       UnderReview,
                allowed_to: vec![Interview, Rejected, Withdrawn],
            },
            TransitionRule {
                from:       Interview,
                allowed_to: vec![Offered, Rejected, Withdrawn],
            },
            TransitionRule {
                from:       Offered,
                allowed_to: vec![Accepted, Rejected, Withdrawn],
            },
            TransitionRule {
                from:       Rejected,
                allowed_to: vec![],
            },
            TransitionRule {
                from:       Accepted,
                allowed_to: vec![],
            },
            TransitionRule {
                from:       Withdrawn,
                allowed_to: vec![],
            },
        ])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ApplicationStatus;

    fn sm() -> StateMachine { StateMachine::default() }

    // -- Valid forward transitions -------------------------------------------

    #[test]
    fn draft_to_submitted_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Draft, ApplicationStatus::Submitted,)
                .is_ok()
        );
    }

    #[test]
    fn submitted_to_under_review_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Submitted, ApplicationStatus::UnderReview,)
                .is_ok()
        );
    }

    #[test]
    fn under_review_to_interview_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::UnderReview, ApplicationStatus::Interview,)
                .is_ok()
        );
    }

    #[test]
    fn interview_to_offered_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Interview, ApplicationStatus::Offered,)
                .is_ok()
        );
    }

    #[test]
    fn offered_to_accepted_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Offered, ApplicationStatus::Accepted,)
                .is_ok()
        );
    }

    // -- Withdrawal from any non-terminal state ------------------------------

    #[test]
    fn draft_to_withdrawn_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Draft, ApplicationStatus::Withdrawn,)
                .is_ok()
        );
    }

    #[test]
    fn submitted_to_withdrawn_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Submitted, ApplicationStatus::Withdrawn,)
                .is_ok()
        );
    }

    #[test]
    fn offered_to_withdrawn_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Offered, ApplicationStatus::Withdrawn,)
                .is_ok()
        );
    }

    // -- Rejection from review stages ----------------------------------------

    #[test]
    fn submitted_to_rejected_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Submitted, ApplicationStatus::Rejected,)
                .is_ok()
        );
    }

    #[test]
    fn interview_to_rejected_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Interview, ApplicationStatus::Rejected,)
                .is_ok()
        );
    }

    #[test]
    fn offered_to_rejected_is_valid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Offered, ApplicationStatus::Rejected,)
                .is_ok()
        );
    }

    // -- Invalid transitions -------------------------------------------------

    #[test]
    fn rejected_is_terminal() {
        assert!(sm().is_terminal(ApplicationStatus::Rejected));
        assert!(
            sm().validate_transition(ApplicationStatus::Rejected, ApplicationStatus::Draft,)
                .is_err()
        );
    }

    #[test]
    fn accepted_is_terminal() {
        assert!(sm().is_terminal(ApplicationStatus::Accepted));
        assert!(
            sm().validate_transition(ApplicationStatus::Accepted, ApplicationStatus::Submitted,)
                .is_err()
        );
    }

    #[test]
    fn withdrawn_is_terminal() {
        assert!(sm().is_terminal(ApplicationStatus::Withdrawn));
        assert!(
            sm().validate_transition(ApplicationStatus::Withdrawn, ApplicationStatus::Draft,)
                .is_err()
        );
    }

    #[test]
    fn draft_to_offered_is_invalid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Draft, ApplicationStatus::Offered,)
                .is_err()
        );
    }

    #[test]
    fn draft_to_accepted_is_invalid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Draft, ApplicationStatus::Accepted,)
                .is_err()
        );
    }

    #[test]
    fn submitted_to_offered_is_invalid() {
        assert!(
            sm().validate_transition(ApplicationStatus::Submitted, ApplicationStatus::Offered,)
                .is_err()
        );
    }

    // -- Allowed transitions listing -----------------------------------------

    #[test]
    fn allowed_transitions_for_draft() {
        let allowed = sm().allowed_transitions(ApplicationStatus::Draft);
        assert_eq!(allowed.len(), 2);
        assert!(allowed.contains(&ApplicationStatus::Submitted));
        assert!(allowed.contains(&ApplicationStatus::Withdrawn));
    }

    #[test]
    fn allowed_transitions_for_terminal_is_empty() {
        assert!(
            sm().allowed_transitions(ApplicationStatus::Rejected)
                .is_empty()
        );
    }

    // -- Custom rules --------------------------------------------------------

    #[test]
    fn custom_rules_override_defaults() {
        let custom = StateMachine::with_rules(vec![TransitionRule {
            from:       ApplicationStatus::Rejected,
            allowed_to: vec![ApplicationStatus::Draft],
        }]);

        // Custom rule allows Rejected -> Draft.
        assert!(
            custom
                .validate_transition(ApplicationStatus::Rejected, ApplicationStatus::Draft,)
                .is_ok()
        );

        // But Draft -> Submitted is no longer defined.
        assert!(
            custom
                .validate_transition(ApplicationStatus::Draft, ApplicationStatus::Submitted,)
                .is_err()
        );
    }
}
