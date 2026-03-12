//! Integration test: prompt injection via external content.

use rara_kernel::guard::pipeline::{GuardPipeline, GuardVerdict};
use rara_kernel::session::SessionKey;

#[test]
fn xiaohongshu_prompt_injection_blocked() {
    let pipeline = GuardPipeline::new();
    let session = SessionKey::new();

    pipeline.post_execute(&session, "web_fetch");

    let args = serde_json::json!({ "command": "rm -rf /" });
    let verdict = pipeline.pre_execute(&session, "bash", &args);

    match verdict {
        GuardVerdict::Blocked { layer, reason, .. } => {
            assert_eq!(layer, "taint");
            assert!(reason.contains("ExternalNetwork"));
        }
        GuardVerdict::Pass => panic!("Expected bash to be blocked after web_fetch"),
    }
}

#[test]
fn xiaohongshu_even_safe_command_blocked_after_web() {
    let pipeline = GuardPipeline::new();
    let session = SessionKey::new();

    pipeline.post_execute(&session, "web_fetch");

    let args = serde_json::json!({ "command": "ls -la" });
    let verdict = pipeline.pre_execute(&session, "bash", &args);
    assert!(matches!(
        verdict,
        GuardVerdict::Blocked { layer: "taint", .. }
    ));
}

#[test]
fn direct_user_request_not_blocked() {
    let pipeline = GuardPipeline::new();
    let session = SessionKey::new();

    let args = serde_json::json!({ "command": "ls -la" });
    let verdict = pipeline.pre_execute(&session, "bash", &args);
    assert!(matches!(verdict, GuardVerdict::Pass));
}

#[test]
fn child_session_inherits_taint() {
    let pipeline = GuardPipeline::new();
    let parent = SessionKey::new();
    let child = SessionKey::new();

    pipeline.post_execute(&parent, "web_fetch");
    pipeline.taint_tracker().fork_session(&parent, &child);

    let args = serde_json::json!({ "command": "echo hello" });
    let verdict = pipeline.pre_execute(&child, "bash", &args);
    assert!(matches!(
        verdict,
        GuardVerdict::Blocked { layer: "taint", .. }
    ));
}
