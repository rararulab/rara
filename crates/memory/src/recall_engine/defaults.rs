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

//! Default recall rules seeded when the engine starts with no persisted rules.

use super::types::{EventKind, InjectTarget, RecallAction, RecallRule, Trigger};

/// Return the built-in default recall rules.
///
/// These rules replicate the previously-hardcoded recall behavior:
///
/// 1. **user-profile** -- always inject the user profile into the system prompt
///    (priority 0, runs first).
/// 2. **new-session-context** -- on new/short sessions, search memory for
///    context relevant to the user's message.
/// 3. **post-compaction** -- after history compaction, search memory using the
///    compacted summary to recover lost details.
/// 4. **session-resume** -- when resuming an inactive session, search for
///    relevant context.
pub fn default_rules() -> Vec<RecallRule> {
    vec![
        RecallRule {
            id:       "default-user-profile".to_owned(),
            name:     "user-profile".to_owned(),
            trigger:  Trigger::Always,
            action:   RecallAction::GetProfile,
            inject:   InjectTarget::SystemPrompt,
            priority: 0,
            enabled:  true,
        },
        RecallRule {
            id:       "default-new-session-context".to_owned(),
            name:     "new-session-context".to_owned(),
            trigger:  Trigger::Event {
                kind: EventKind::NewSession,
            },
            action:   RecallAction::Search {
                query_template: "{user_text}".to_owned(),
                limit:          5,
            },
            inject:   InjectTarget::SystemPrompt,
            priority: 5,
            enabled:  true,
        },
        RecallRule {
            id:       "default-post-compaction".to_owned(),
            name:     "post-compaction".to_owned(),
            trigger:  Trigger::Event {
                kind: EventKind::Compaction,
            },
            action:   RecallAction::Search {
                query_template: "{summary}".to_owned(),
                limit:          5,
            },
            inject:   InjectTarget::SystemPrompt,
            priority: 10,
            enabled:  true,
        },
        RecallRule {
            id:       "default-session-resume".to_owned(),
            name:     "session-resume".to_owned(),
            trigger:  Trigger::Event {
                kind: EventKind::SessionResume,
            },
            action:   RecallAction::Search {
                query_template: "{user_text}".to_owned(),
                limit:          3,
            },
            inject:   InjectTarget::SystemPrompt,
            priority: 20,
            enabled:  true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rules_are_all_enabled() {
        for rule in default_rules() {
            assert!(rule.enabled, "rule '{}' should be enabled", rule.name);
        }
    }

    #[test]
    fn default_rules_have_unique_ids() {
        let rules = default_rules();
        let mut ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), rules.len());
    }
}
