use std::time::Duration;

use rara_app::{AppConfig, StartOptions, start_with_options};
use rara_kernel::{
    agent::TurnTrace,
    channel::types::MessageContent,
    identity::{Principal, UserId},
    io::{ChannelSource, InboundMessage, MessageId},
    memory::{FileTapeStore, TapEntry, TapEntryKind, TapeService},
    session::SessionKey,
};
use serde_json::Value;
use tokio::time::{Instant, sleep};
use uuid::Uuid;

const DEFAULT_SOAK_SECS: u64 = 600;
const DEFAULT_TURN_TIMEOUT_SECS: u64 = 180;
const MIN_REQUIRED_TURNS: usize = 6;
const EXPECTED_MIN_READ_FILE_CALLS: usize = 9;
const EARLY_FACT: &str = "3月14号从上海飞东京要2768元";
const RESEARCH_TARGET: &str = "https://github.com/qwibitai/nanoclaw";

fn soak_duration() -> Duration {
    std::env::var("RARA_REAL_TAPE_SOAK_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_SOAK_SECS))
}

fn turn_timeout() -> Duration {
    std::env::var("RARA_REAL_TAPE_TURN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_TURN_TIMEOUT_SECS))
}

fn build_test_message(
    session_key: Option<SessionKey>,
    chat_id: &str,
    text: &str,
) -> InboundMessage {
    InboundMessage {
        id: MessageId::new(),
        source: ChannelSource {
            channel_type:        rara_kernel::channel::types::ChannelType::Internal,
            platform_message_id: None,
            platform_user_id:    "ryan".to_string(),
            platform_chat_id:    Some(chat_id.to_string()),
        },
        user: UserId("ryan".to_string()),
        session_key,
        target_session_key: None,
        content: MessageContent::Text(text.to_string()),
        reply_context: None,
        timestamp: jiff::Timestamp::now(),
        metadata: Default::default(),
    }
}

fn large_file_paths() -> [String; 3] {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        root.join("crates/kernel/src/kernel.rs")
            .display()
            .to_string(),
        root.join("crates/kernel/src/agent.rs")
            .display()
            .to_string(),
        root.join("crates/kernel/src/io.rs").display().to_string(),
    ]
}

fn file_for_turn(turn: usize) -> String {
    let files = large_file_paths();
    files[(turn - 1) % files.len()].clone()
}

fn paged_read_instruction(file: &str, segments: &[(usize, usize)]) -> String {
    let mut instructions = Vec::new();
    for (offset, limit) in segments {
        instructions.push(format!(
            "read-file on {file} with offset {offset} and limit {limit}"
        ));
    }
    instructions.join(", then ")
}

fn build_turn_prompt(turn: usize) -> String {
    if turn == 0 {
        return EARLY_FACT.to_string();
    }

    if turn == 1 {
        return format!(
            "开始把这个库 port 到 Zig：{RESEARCH_TARGET}。先给出一个最小可行的 port \
             方向，包括模块拆分、核心数据结构和第一阶段实现顺序。"
        );
    }

    if turn == 2 {
        let file = file_for_turn(turn);
        return format!(
            "继续做 {target} 的 Zig port。先把相关实现读深一点：{reads}。读完后，用四个短 bullet \
             写出你会在 Zig 里先落地的模块划分，并点出一个必须先抽象出来的子系统。",
            target = RESEARCH_TARGET,
            reads = paged_read_instruction(&file, &[(1, 500), (501, 500), (1001, 500)]),
        );
    }

    if turn == 3 {
        let file = file_for_turn(turn);
        return format!(
            "继续做 {target} 的 Zig port，重点看执行循环和状态推进：{reads}。读完后，用四个短 \
             bullet 说明你会怎样把主执行路径映射到 Zig，并指出一个实现风险。",
            target = RESEARCH_TARGET,
            reads = paged_read_instruction(&file, &[(1, 500), (501, 500), (1001, 500)]),
        );
    }

    if turn == 4 {
        let file = file_for_turn(turn);
        return format!(
            "继续做 {target} 的 Zig port，这轮重点看消息流和 I/O 路径：{reads}。读完后，用四个 短 \
             bullet 写出你会在 Zig 里如何表示数据流，并点出一个热点路径。",
            target = RESEARCH_TARGET,
            reads = paged_read_instruction(&file, &[(1, 500), (501, 500), (1001, 500)]),
        );
    }

    if turn == 5 {
        return format!(
            "port 检查点 turn {turn}：基于到目前为止看到的内容，概括 {target} 做 Zig port 时最 \
             重要的架构结论和风险，用一个紧凑段落写出来。",
            target = RESEARCH_TARGET,
        );
    }

    match (turn - 6) % 5 {
        0 => format!(
            "继续做 {target} 的 Zig port。先深入读实现：{reads}。然后用三个短 bullet 写出这部 \
             分代码在 Zig 里的职责映射，并精确提到一个源码里的标识符。",
            target = RESEARCH_TARGET,
            reads =
                paged_read_instruction(&file_for_turn(turn), &[(1, 500), (501, 500), (1001, 500)]),
        ),
        1 => format!(
            "继续做 {target} 的 Zig port。先分别读两组实现：{left_reads}，然后 {right_reads}。 \
             最后用三个短 bullet 比较这两部分设计，并指出一个 Zig port 时的共通难点。",
            target = RESEARCH_TARGET,
            left_reads = paged_read_instruction(&file_for_turn(turn), &[(1, 400), (401, 400)]),
            right_reads = paged_read_instruction(&file_for_turn(turn + 1), &[(1, 400), (401, 400)]),
        ),
        2 => format!(
            "port 笔记 turn {turn}：把目前对 {target} 的迁移思路压成一个紧凑段落，重点写清楚哪 \
             几个点会直接影响 Zig port 的拆分顺序。",
            target = RESEARCH_TARGET,
        ),
        3 => format!(
            "port 检查点 turn {turn}：用三个短 bullet 总结目前最关键的迁移发现，然后补一个下一步 \
             最值得深挖的问题，围绕 {target} 的 Zig port 来写。",
            target = RESEARCH_TARGET,
        ),
        _ => format!(
            "继续做 {target} 的 Zig port。先读这一组实现：{reads}。读完后，用三个短 bullet 写出 \
             两个核心子系统和一个近期最值得警惕的实现细节，结论要服务于 Zig port。",
            target = RESEARCH_TARGET,
            reads =
                paged_read_instruction(&file_for_turn(turn), &[(1, 500), (501, 500), (1001, 500)]),
        ),
    }
}

fn trace_has_tool_call(trace: &TurnTrace, tool_name: &str) -> bool {
    trace
        .iterations
        .iter()
        .flat_map(|iter| &iter.tool_calls)
        .any(|call| call.name == tool_name)
}

fn trace_tool_call_count(trace: &TurnTrace, tool_name: &str) -> usize {
    trace
        .iterations
        .iter()
        .flat_map(|iter| &iter.tool_calls)
        .filter(|call| call.name == tool_name)
        .count()
}

fn trace_has_tape_action(trace: &TurnTrace, action: &str) -> bool {
    trace
        .iterations
        .iter()
        .flat_map(|iter| &iter.tool_calls)
        .any(|call| {
            call.name == "tape"
                && call
                    .arguments
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == action)
        })
}

fn trace_text_preview(trace: &TurnTrace) -> String {
    trace
        .iterations
        .last()
        .map(|iter| iter.text_preview.clone())
        .unwrap_or_default()
}

fn usage_from_entry(entry: &TapEntry) -> Option<(u64, u64, u64)> {
    if entry.kind != TapEntryKind::Event {
        return None;
    }
    let event_name = entry.payload.get("name").and_then(Value::as_str)?;
    if !matches!(event_name, "run" | "llm.run") {
        return None;
    }
    let usage = entry.payload.get("data")?.get("usage")?;
    Some((
        usage.get("prompt_tokens")?.as_u64()?,
        usage.get("completion_tokens")?.as_u64()?,
        usage.get("total_tokens")?.as_u64()?,
    ))
}

async fn collect_usage_rows(
    workspace_root: &std::path::Path,
    session_key: SessionKey,
) -> Vec<(u64, u64, u64)> {
    let tape_service = TapeService::new(
        FileTapeStore::new(rara_paths::memory_dir(), workspace_root)
            .await
            .expect("tape store should open"),
    );
    tape_service
        .entries(&session_key.to_string())
        .await
        .expect("session tape should load")
        .iter()
        .filter_map(usage_from_entry)
        .collect()
}

async fn wait_for_turn_count(
    handle: &rara_kernel::handle::KernelHandle,
    session_key: SessionKey,
    expected_turns: usize,
) {
    let deadline = Instant::now() + turn_timeout();
    loop {
        let traces = handle.get_process_turns(session_key);
        if traces.len() >= expected_turns {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for turn {expected_turns} in session {session_key}; \
             current_turns={} latest_trace={:?}",
            traces.len(),
            traces.last()
        );
        sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
#[ignore = "uses the real LLM provider configured in config.yaml and runs a long soak"]
async fn real_model_soak_test_survives_long_tape_pressure() {
    common_telemetry::logging::init_default_ut_logging();
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&workspace_root).expect("should switch to workspace root");

    let mut config = AppConfig::new().expect("config should load");
    config.http.bind_address = "127.0.0.1:0".to_string();
    config.grpc.bind_address = "127.0.0.1:0".to_string();
    config.grpc.server_address = "127.0.0.1:0".to_string();
    config.gateway = None;
    config.telegram = None;
    config.symphony = None;

    let mut app = start_with_options(config, StartOptions::default())
        .await
        .expect("app should start");
    let handle = app
        .kernel_handle
        .take()
        .expect("kernel handle should be available");

    let started_at = Instant::now();
    let deadline = started_at + soak_duration();
    let chat_id = format!("real-tape-soak-{}", Uuid::new_v4());
    let initial_prompt = build_turn_prompt(0);
    let principal = Principal::lookup("ryan".to_string());
    let session_key = handle
        .spawn_named("rara", initial_prompt, principal, None)
        .await
        .expect("should spawn rara session");
    wait_for_turn_count(&handle, session_key, 1).await;
    let mut turn_index = 0usize;
    let mut saw_read_file = false;
    let mut read_file_calls = 0usize;
    let mut saw_anchor = false;
    let mut saw_search = false;

    while turn_index < MIN_REQUIRED_TURNS || Instant::now() < deadline {
        if turn_index == 0 {
            turn_index += 1;
            continue;
        }
        let prompt = build_turn_prompt(turn_index);
        handle
            .submit_message(build_test_message(Some(session_key), &chat_id, &prompt))
            .expect("message should submit");
        wait_for_turn_count(&handle, session_key, turn_index + 1).await;

        let traces = handle.get_process_turns(session_key);
        let latest = traces
            .last()
            .unwrap_or_else(|| panic!("missing trace for turn {turn_index}"));

        assert!(
            latest.success,
            "turn {turn_index} failed: {:?}",
            latest.error
        );
        assert!(
            latest.error.is_none(),
            "turn {turn_index} recorded an error: {:?}",
            latest.error
        );
        let latest_debug = format!("{latest:?}");
        assert!(
            !latest_debug.contains("Context window"),
            "turn {turn_index} hit context pressure failure: {latest_debug}"
        );

        saw_read_file |= trace_has_tool_call(latest, "read-file");
        read_file_calls += trace_tool_call_count(latest, "read-file");
        saw_anchor |= trace_has_tape_action(latest, "anchor");
        saw_search |= trace_has_tape_action(latest, "search");
        turn_index += 1;
    }

    let final_prompt =
        "最后确认一下，上海飞东京要多少钱？如果你不确定，就先验证再回答。只回答价格。";
    handle
        .submit_message(build_test_message(
            Some(session_key),
            &chat_id,
            final_prompt,
        ))
        .expect("final recall message should submit");
    wait_for_turn_count(&handle, session_key, turn_index + 1).await;

    let traces = handle.get_process_turns(session_key);
    let final_trace = traces
        .last()
        .unwrap_or_else(|| panic!("missing final recall trace after turn {turn_index}"));
    assert!(
        final_trace.success,
        "final recall failed: {:?}",
        final_trace.error
    );
    assert!(
        final_trace.error.is_none(),
        "final recall recorded an error: {:?}",
        final_trace.error
    );
    saw_search |= trace_has_tape_action(final_trace, "search");
    let final_preview: String = trace_text_preview(final_trace);
    assert!(
        final_preview.contains("2768"),
        "final recall should return the Shanghai -> Tokyo price, got: {final_preview}"
    );
    turn_index += 1;

    eprintln!(
        "soak completed: turns={turn_index} elapsed_secs={} read_file_calls={read_file_calls} \
         saw_read_file={saw_read_file} saw_anchor={saw_anchor} saw_search={saw_search} \
         latest_preview={latest_preview}",
        started_at.elapsed().as_secs(),
        latest_preview = final_preview,
    );

    let usage_rows = collect_usage_rows(&workspace_root, session_key).await;
    let total_prompt_tokens: u64 = usage_rows.iter().map(|(prompt, ..)| *prompt).sum();
    let total_completion_tokens: u64 = usage_rows
        .iter()
        .map(|(_, completion, _)| *completion)
        .sum();
    let total_tokens: u64 = usage_rows.iter().map(|(_, _, total)| *total).sum();
    eprintln!("token usage by llm.run event:");
    for (index, (prompt, completion, total)) in usage_rows.iter().enumerate() {
        eprintln!(
            "  run[{index}] prompt_tokens={prompt} completion_tokens={completion} \
             total_tokens={total}"
        );
    }
    eprintln!(
        "token usage totals: prompt_tokens={total_prompt_tokens} \
         completion_tokens={total_completion_tokens} total_tokens={total_tokens}"
    );

    assert!(
        turn_index >= MIN_REQUIRED_TURNS,
        "soak should execute at least {MIN_REQUIRED_TURNS} turns"
    );
    assert!(saw_read_file, "soak never observed read-file usage");
    assert!(
        read_file_calls >= EXPECTED_MIN_READ_FILE_CALLS,
        "soak only observed {read_file_calls} read-file calls"
    );
    assert!(saw_anchor, "soak never observed tape.anchor usage");
    assert!(saw_search, "soak never observed tape.search usage");

    app.shutdown();
}
