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

//! Recall Strategy Engine -- evaluates rules and executes memory actions.

use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::MemoryManager;

use super::interpolation::interpolate;
use super::types::{
    InjectionPayload, MatchedAction, RecallAction, RecallContext, RecallRule, RecallRuleUpdate,
    Trigger,
};

/// Agent-configurable recall strategy engine.
///
/// Holds a set of rules that are evaluated against a [`RecallContext`] on
/// each turn. Matched rules produce actions that query the
/// [`MemoryManager`], yielding injection payloads ready for prompt assembly.
pub struct RecallStrategyEngine {
    rules: RwLock<Vec<RecallRule>>,
}

impl RecallStrategyEngine {
    /// Create a new engine pre-loaded with the given rules.
    pub fn new(rules: Vec<RecallRule>) -> Self {
        Self {
            rules: RwLock::new(rules),
        }
    }

    /// Evaluate all enabled rules against the context.
    ///
    /// Returns matched actions sorted by ascending priority (lower = higher
    /// priority, executed first).
    pub async fn evaluate(&self, ctx: &RecallContext) -> Vec<MatchedAction> {
        let rules = self.rules.read().await;
        let mut matched: Vec<MatchedAction> = rules
            .iter()
            .filter(|r| r.enabled)
            .filter(|r| evaluate_trigger(&r.trigger, ctx))
            .map(|r| MatchedAction {
                rule_name: r.name.clone(),
                action: r.action.clone(),
                inject: r.inject,
                priority: r.priority,
            })
            .collect();

        matched.sort_by_key(|m| m.priority);
        matched
    }

    /// Execute matched actions using the memory manager.
    ///
    /// Returns injection payloads containing the recalled content and
    /// target location for prompt injection.
    pub async fn execute(
        &self,
        matched: &[MatchedAction],
        mm: &MemoryManager,
        ctx: &RecallContext,
    ) -> Vec<InjectionPayload> {
        let mut payloads = Vec::new();

        for action in matched {
            match &action.action {
                RecallAction::Search {
                    query_template,
                    limit,
                } => {
                    let query = interpolate(query_template, ctx);
                    match mm.search(&query, *limit).await {
                        Ok(results) if !results.is_empty() => {
                            let content: String = results
                                .iter()
                                .map(|r| format!("- [{:?}] {}", r.source, r.content))
                                .collect::<Vec<_>>()
                                .join("\n");
                            payloads.push(InjectionPayload {
                                rule_name: action.rule_name.clone(),
                                target: action.inject,
                                content,
                            });
                            info!(
                                rule = %action.rule_name,
                                hits = results.len(),
                                "recall search returned results"
                            );
                        }
                        Ok(_) => {
                            // No results, skip.
                        }
                        Err(e) => {
                            warn!(
                                rule = %action.rule_name,
                                error = %e,
                                "recall search failed"
                            );
                        }
                    }
                }
                RecallAction::DeepRecall { query_template } => {
                    let query = interpolate(query_template, ctx);
                    match mm.deep_recall(&query).await {
                        Ok(text) if !text.is_empty() => {
                            payloads.push(InjectionPayload {
                                rule_name: action.rule_name.clone(),
                                target: action.inject,
                                content: text,
                            });
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!(
                                rule = %action.rule_name,
                                error = %e,
                                "recall deep_recall failed"
                            );
                        }
                    }
                }
                RecallAction::GetProfile => match mm.get_user_profile().await {
                    Ok(facts) if !facts.is_empty() => {
                        let content: String = facts
                            .iter()
                            .map(|m| format!("- {}", m.memory))
                            .collect::<Vec<_>>()
                            .join("\n");
                        payloads.push(InjectionPayload {
                            rule_name: action.rule_name.clone(),
                            target: action.inject,
                            content,
                        });
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(
                            rule = %action.rule_name,
                            error = %e,
                            "recall get_user_profile failed"
                        );
                    }
                },
            }
        }

        payloads
    }

    /// Convenience: evaluate + execute in one call.
    pub async fn run(
        &self,
        ctx: &RecallContext,
        mm: &MemoryManager,
    ) -> Vec<InjectionPayload> {
        let matched = self.evaluate(ctx).await;
        if matched.is_empty() {
            return vec![];
        }
        self.execute(&matched, mm, ctx).await
    }

    // -- CRUD for rules (used by agent tools) ---------------------------------

    /// Add a new rule.
    pub async fn add_rule(&self, rule: RecallRule) {
        self.rules.write().await.push(rule);
    }

    /// List all rules.
    pub async fn list_rules(&self) -> Vec<RecallRule> {
        self.rules.read().await.clone()
    }

    /// Update an existing rule. Returns true if found and updated.
    pub async fn update_rule(&self, id: &str, update: RecallRuleUpdate) -> bool {
        let mut rules = self.rules.write().await;
        if let Some(rule) = rules.iter_mut().find(|r| r.id == id) {
            if let Some(trigger) = update.trigger {
                rule.trigger = trigger;
            }
            if let Some(action) = update.action {
                rule.action = action;
            }
            if let Some(inject) = update.inject {
                rule.inject = inject;
            }
            if let Some(priority) = update.priority {
                rule.priority = priority;
            }
            if let Some(enabled) = update.enabled {
                rule.enabled = enabled;
            }
            true
        } else {
            false
        }
    }

    /// Remove a rule by id. Returns true if found and removed.
    pub async fn remove_rule(&self, id: &str) -> bool {
        let mut rules = self.rules.write().await;
        let len_before = rules.len();
        rules.retain(|r| r.id != id);
        rules.len() < len_before
    }
}

/// Recursively evaluate a trigger tree against the given context.
fn evaluate_trigger(trigger: &Trigger, ctx: &RecallContext) -> bool {
    match trigger {
        Trigger::KeywordMatch { keywords } => {
            let lower = ctx.user_text.to_lowercase();
            keywords.iter().any(|kw| lower.contains(&kw.to_lowercase()))
        }
        Trigger::Event { kind } => ctx.events.contains(kind),
        Trigger::EveryNTurns { n } => {
            if *n == 0 {
                return false;
            }
            ctx.turn_count % (*n as usize) == 0
        }
        Trigger::InactivityGt { seconds } => ctx.elapsed_since_last_secs > *seconds,
        Trigger::And { conditions } => conditions.iter().all(|c| evaluate_trigger(c, ctx)),
        Trigger::Or { conditions } => conditions.iter().any(|c| evaluate_trigger(c, ctx)),
        Trigger::Not { condition } => !evaluate_trigger(condition, ctx),
        Trigger::Always => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{EventKind, InjectTarget};

    fn make_ctx() -> RecallContext {
        RecallContext {
            user_text: "hello world".to_owned(),
            turn_count: 6,
            events: vec![],
            elapsed_since_last_secs: 0,
            summary: None,
            session_topic: None,
        }
    }

    fn make_rule(name: &str, trigger: Trigger, priority: u16) -> RecallRule {
        RecallRule {
            id: name.to_owned(),
            name: name.to_owned(),
            trigger,
            action: RecallAction::Search {
                query_template: "{user_text}".to_owned(),
                limit: 5,
            },
            inject: InjectTarget::SystemPrompt,
            priority,
            enabled: true,
        }
    }

    #[test]
    fn test_keyword_match() {
        let ctx = make_ctx();
        let trigger = Trigger::KeywordMatch {
            keywords: vec!["hello".to_owned(), "foo".to_owned()],
        };
        assert!(evaluate_trigger(&trigger, &ctx));
    }

    #[test]
    fn test_keyword_no_match() {
        let ctx = make_ctx();
        let trigger = Trigger::KeywordMatch {
            keywords: vec!["rust".to_owned(), "programming".to_owned()],
        };
        assert!(!evaluate_trigger(&trigger, &ctx));
    }

    #[test]
    fn test_keyword_case_insensitive() {
        let ctx = RecallContext {
            user_text: "Hello World".to_owned(),
            ..make_ctx()
        };
        let trigger = Trigger::KeywordMatch {
            keywords: vec!["HELLO".to_owned()],
        };
        assert!(evaluate_trigger(&trigger, &ctx));
    }

    #[test]
    fn test_event_trigger() {
        let ctx = RecallContext {
            events: vec![EventKind::Compaction],
            ..make_ctx()
        };
        let trigger = Trigger::Event {
            kind: EventKind::Compaction,
        };
        assert!(evaluate_trigger(&trigger, &ctx));
    }

    #[test]
    fn test_event_trigger_no_match() {
        let ctx = RecallContext {
            events: vec![EventKind::NewSession],
            ..make_ctx()
        };
        let trigger = Trigger::Event {
            kind: EventKind::Compaction,
        };
        assert!(!evaluate_trigger(&trigger, &ctx));
    }

    #[test]
    fn test_every_n_turns() {
        // turn_count = 6, n = 3 => 6 % 3 == 0 => true
        let ctx = make_ctx();
        assert!(evaluate_trigger(&Trigger::EveryNTurns { n: 3 }, &ctx));

        // turn_count = 6, n = 4 => 6 % 4 == 2 => false
        assert!(!evaluate_trigger(&Trigger::EveryNTurns { n: 4 }, &ctx));
    }

    #[test]
    fn test_every_n_turns_zero() {
        let ctx = make_ctx();
        assert!(!evaluate_trigger(&Trigger::EveryNTurns { n: 0 }, &ctx));
    }

    #[test]
    fn test_inactivity_gt() {
        let ctx = RecallContext {
            elapsed_since_last_secs: 120,
            ..make_ctx()
        };
        assert!(evaluate_trigger(
            &Trigger::InactivityGt { seconds: 60 },
            &ctx
        ));
        assert!(!evaluate_trigger(
            &Trigger::InactivityGt { seconds: 120 },
            &ctx
        ));
    }

    #[test]
    fn test_and_combinator() {
        let ctx = RecallContext {
            events: vec![EventKind::Compaction],
            ..make_ctx()
        };
        let trigger = Trigger::And {
            conditions: vec![
                Trigger::KeywordMatch {
                    keywords: vec!["hello".to_owned()],
                },
                Trigger::Event {
                    kind: EventKind::Compaction,
                },
            ],
        };
        assert!(evaluate_trigger(&trigger, &ctx));

        // Fails when one condition is false.
        let trigger_fail = Trigger::And {
            conditions: vec![
                Trigger::KeywordMatch {
                    keywords: vec!["hello".to_owned()],
                },
                Trigger::Event {
                    kind: EventKind::NewSession,
                },
            ],
        };
        assert!(!evaluate_trigger(&trigger_fail, &ctx));
    }

    #[test]
    fn test_or_combinator() {
        let ctx = make_ctx();
        let trigger = Trigger::Or {
            conditions: vec![
                Trigger::KeywordMatch {
                    keywords: vec!["nope".to_owned()],
                },
                Trigger::KeywordMatch {
                    keywords: vec!["hello".to_owned()],
                },
            ],
        };
        assert!(evaluate_trigger(&trigger, &ctx));

        let trigger_none = Trigger::Or {
            conditions: vec![
                Trigger::KeywordMatch {
                    keywords: vec!["nope".to_owned()],
                },
                Trigger::KeywordMatch {
                    keywords: vec!["also_nope".to_owned()],
                },
            ],
        };
        assert!(!evaluate_trigger(&trigger_none, &ctx));
    }

    #[test]
    fn test_not_combinator() {
        let ctx = make_ctx();
        let trigger = Trigger::Not {
            condition: Box::new(Trigger::KeywordMatch {
                keywords: vec!["nope".to_owned()],
            }),
        };
        assert!(evaluate_trigger(&trigger, &ctx));

        let trigger_neg = Trigger::Not {
            condition: Box::new(Trigger::KeywordMatch {
                keywords: vec!["hello".to_owned()],
            }),
        };
        assert!(!evaluate_trigger(&trigger_neg, &ctx));
    }

    #[test]
    fn test_always_trigger() {
        let ctx = make_ctx();
        assert!(evaluate_trigger(&Trigger::Always, &ctx));
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let engine = RecallStrategyEngine::new(vec![
            make_rule("low-priority", Trigger::Always, 100),
            make_rule("high-priority", Trigger::Always, 10),
            make_rule("mid-priority", Trigger::Always, 50),
        ]);
        let ctx = make_ctx();
        let matched = engine.evaluate(&ctx).await;
        assert_eq!(matched.len(), 3);
        assert_eq!(matched[0].rule_name, "high-priority");
        assert_eq!(matched[1].rule_name, "mid-priority");
        assert_eq!(matched[2].rule_name, "low-priority");
    }

    #[tokio::test]
    async fn test_disabled_rules_skipped() {
        let mut rule = make_rule("disabled", Trigger::Always, 10);
        rule.enabled = false;
        let engine = RecallStrategyEngine::new(vec![rule]);
        let ctx = make_ctx();
        let matched = engine.evaluate(&ctx).await;
        assert!(matched.is_empty());
    }

    #[tokio::test]
    async fn test_add_and_list_rules() {
        let engine = RecallStrategyEngine::new(vec![]);
        assert!(engine.list_rules().await.is_empty());

        engine
            .add_rule(make_rule("new-rule", Trigger::Always, 50))
            .await;
        let rules = engine.list_rules().await;
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "new-rule");
    }

    #[tokio::test]
    async fn test_update_rule() {
        let engine = RecallStrategyEngine::new(vec![make_rule("r1", Trigger::Always, 50)]);

        let updated = engine
            .update_rule(
                "r1",
                RecallRuleUpdate {
                    priority: Some(10),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await;
        assert!(updated);

        let rules = engine.list_rules().await;
        assert_eq!(rules[0].priority, 10);
        assert!(!rules[0].enabled);

        // Non-existent rule.
        let not_found = engine
            .update_rule("nonexistent", RecallRuleUpdate::default())
            .await;
        assert!(!not_found);
    }

    #[tokio::test]
    async fn test_remove_rule() {
        let engine = RecallStrategyEngine::new(vec![
            make_rule("r1", Trigger::Always, 50),
            make_rule("r2", Trigger::Always, 60),
        ]);

        assert!(engine.remove_rule("r1").await);
        assert_eq!(engine.list_rules().await.len(), 1);
        assert!(!engine.remove_rule("r1").await); // already removed
    }
}
