# Cascade Viewer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace KernelTop's flat TurnTraceTree with a Cascade Viewer that visualizes the ReAct loop (Input → Thought → Action → Observation → Response) with real-time streaming for active processes.

**Architecture:** Extend backend `TurnTrace`/`IterationTrace` structs with `input_text` and `reasoning_text` fields. Add a per-process WebSocket stream endpoint. Build `CascadeViewer` React component using shadcn/ui that combines polled history with live StreamEvent data.

**Tech Stack:** Rust (axum WebSocket, kernel StreamHub), React 19, TypeScript, shadcn/ui (Collapsible, Badge, ScrollArea), TanStack Query, lucide-react icons.

---

## Task 1: Extend Backend Trace Types

**Files:**
- Modify: `crates/core/kernel/src/agent_turn.rs:69-90` (struct definitions)

**Step 1: Add fields to IterationTrace and TurnTrace**

In `agent_turn.rs`, add `reasoning_text` to `IterationTrace` and `input_text` to `TurnTrace`:

```rust
/// Trace of a single LLM iteration within a turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IterationTrace {
    pub index: usize,
    pub first_token_ms: Option<u64>,
    pub stream_ms: u64,
    /// First 200 chars of accumulated text.
    pub text_preview: String,
    /// Full accumulated text for this iteration (the agent's "thinking").
    pub reasoning_text: Option<String>,
    pub tool_calls: Vec<ToolCallTrace>,
}

/// Complete trace of a single agent turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnTrace {
    pub duration_ms: u64,
    pub model: String,
    /// The user message that triggered this turn.
    pub input_text: Option<String>,
    pub iterations: Vec<IterationTrace>,
    pub final_text_len: usize,
    pub total_tool_calls: usize,
    pub success: bool,
    pub error: Option<String>,
}
```

**Step 2: Update all IterationTrace construction sites**

There are 2 places `IterationTrace` is constructed in `agent_turn.rs`:

1. **Terminal response** (line ~365): No tool calls — this is the final "Response". Store full text:
```rust
iteration_traces.push(IterationTrace {
    index: iteration,
    first_token_ms,
    stream_ms,
    text_preview,
    reasoning_text: Some(accumulated_text.clone()),
    tool_calls: vec![],
});
```

2. **Tool call iteration** (line ~537): Has tool calls — this is a "Thought" before actions. Store full text:
```rust
iteration_traces.push(IterationTrace {
    index: iteration,
    first_token_ms,
    stream_ms,
    text_preview,
    reasoning_text: if accumulated_text.is_empty() { None } else { Some(accumulated_text.clone()) },
    tool_calls: tool_call_traces,
});
```

**Step 3: Update all TurnTrace construction sites**

There are 3 places `TurnTrace` is constructed. The function `run_inline_agent_loop` receives `user_text: String` as parameter. We need to capture it.

Add a `let input_text = user_text.clone();` near the top of the function (before `user_text` is moved into the message builder at line ~170), then use it in all 3 TurnTrace constructions:

1. **Terminal response** (line ~372):
```rust
let trace = TurnTrace {
    duration_ms: turn_start.elapsed().as_millis() as u64,
    model: model.clone(),
    input_text: Some(input_text.clone()),
    iterations: iteration_traces,
    // ... rest unchanged
};
```

2. **Max iterations exhausted** (line ~553):
```rust
let trace = TurnTrace {
    duration_ms: turn_start.elapsed().as_millis() as u64,
    model: model.clone(),
    input_text: Some(input_text.clone()),
    iterations: iteration_traces,
    // ... rest unchanged
};
```

3. **Error path** — search for any other `TurnTrace { ... }` construction and add `input_text`.

**Step 4: Fix tests**

Any test constructing `IterationTrace` or `TurnTrace` needs the new fields. Search tests in `agent_turn.rs` for struct literals and add `reasoning_text: None` / `input_text: None`.

**Step 5: Verify compilation**

Run: `cargo check -p rara-kernel`
Expected: PASS (all struct sites updated)

**Step 6: Run kernel tests**

Run: `cargo test -p rara-kernel -- agent_turn`
Expected: All agent_turn tests pass.

**Step 7: Commit**

```bash
git add crates/core/kernel/src/agent_turn.rs
git commit -m "feat(kernel): add reasoning_text and input_text to trace types"
```

---

## Task 2: Add Per-Process Stream WebSocket Endpoint

**Files:**
- Modify: `crates/extensions/backend-admin/src/kernel/router.rs`
- Modify: `crates/extensions/backend-admin/Cargo.toml` (if `axum` ws feature not already enabled)

**Step 1: Check axum WebSocket feature**

Run: `grep -r "axum" crates/extensions/backend-admin/Cargo.toml`

If `ws` feature is not enabled, add it. The backend-admin crate likely already uses axum but may not have `ws` feature.

**Step 2: Add WebSocket handler to router**

In `crates/extensions/backend-admin/src/kernel/router.rs`, add:

```rust
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};

// In kernel_routes():
.route(
    "/api/v1/kernel/processes/{agent_id}/stream",
    get(stream_process),
)
```

**Step 3: Implement stream_process handler**

```rust
async fn stream_process(
    State(kernel): State<Arc<Kernel>>,
    Path(agent_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, ProblemDetails> {
    let aid = rara_kernel::process::AgentId(
        uuid::Uuid::parse_str(&agent_id)
            .map_err(|e| ProblemDetails::bad_request(format!("invalid agent_id: {e}")))?,
    );

    // Get the process's session_id for StreamHub subscription
    let process = kernel
        .process_table()
        .get(aid)
        .ok_or_else(|| ProblemDetails::not_found("Process Not Found", format!("process not found: {agent_id}")))?;
    let session_id = process.session_id.clone();

    let stream_hub = kernel.stream_hub().clone();

    Ok(ws.on_upgrade(move |socket| handle_process_stream(socket, stream_hub, session_id)))
}

async fn handle_process_stream(
    mut socket: WebSocket,
    stream_hub: Arc<rara_kernel::io::stream::StreamHub>,
    session_id: rara_kernel::process::SessionId,
) {
    use tokio::time::{interval, Duration};

    // Poll for active streams on this session.
    // The stream may not exist yet if the process is idle, so we poll.
    let mut poll_interval = interval(Duration::from_millis(200));
    let mut receivers: Vec<tokio::sync::broadcast::Receiver<rara_kernel::io::stream::StreamEvent>> = Vec::new();

    loop {
        // If no active receivers, try to subscribe
        if receivers.is_empty() {
            let subs = stream_hub.subscribe_session(&session_id);
            receivers = subs.into_iter().map(|(_, rx)| rx).collect();
            if receivers.is_empty() {
                // No active stream — wait and retry
                tokio::select! {
                    _ = poll_interval.tick() => continue,
                    msg = socket.recv() => {
                        // Client disconnected or sent close
                        match msg {
                            Some(Ok(Message::Close(_))) | None => return,
                            _ => continue,
                        }
                    }
                }
            }
        }

        // Drain from all receivers
        let mut got_event = false;
        for rx in &mut receivers {
            match rx.try_recv() {
                Ok(event) => {
                    got_event = true;
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        return; // Client disconnected
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    // Stream ended — send done event and clean up
                    let _ = socket
                        .send(Message::Text(r#"{"type":"done"}"#.into()))
                        .await;
                    return;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "process stream subscriber lagged");
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            }
        }

        if !got_event {
            // Brief sleep to avoid busy-loop
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {},
                msg = socket.recv() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => return,
                        _ => {}
                    }
                }
            }
        }
    }
}
```

**Step 4: Verify compilation**

Run: `cargo check -p rara-backend-admin`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/extensions/backend-admin/src/kernel/router.rs crates/extensions/backend-admin/Cargo.toml
git commit -m "feat(backend): add per-process WebSocket stream endpoint"
```

---

## Task 3: Build CascadeViewer Frontend Component

**Files:**
- Create: `web/src/components/CascadeViewer.tsx`

**Step 1: Create the CascadeViewer component**

This component takes `TurnTrace[]` and renders the cascade tree. Create `web/src/components/CascadeViewer.tsx`:

```tsx
import { useState } from "react";
import {
  Brain,
  Bot,
  ChevronRight,
  ChevronDown,
  Eye,
  Loader2,
  MessageSquare,
  Wrench,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";

// ---------------------------------------------------------------------------
// Types (must match backend)
// ---------------------------------------------------------------------------

interface ToolCallTrace {
  name: string;
  id: string;
  duration_ms: number;
  success: boolean;
  arguments: Record<string, unknown>;
  result_preview: string;
  error: string | null;
}

interface IterationTrace {
  index: number;
  first_token_ms: number | null;
  stream_ms: number;
  text_preview: string;
  reasoning_text: string | null;
  tool_calls: ToolCallTrace[];
}

interface TurnTrace {
  duration_ms: number;
  model: string;
  input_text: string | null;
  iterations: IterationTrace[];
  final_text_len: number;
  total_tool_calls: number;
  success: boolean;
  error: string | null;
}

// Streaming node (from WebSocket)
export interface StreamingNode {
  type: "thought" | "action" | "observation";
  key: string;
  text?: string;
  toolName?: string;
  toolId?: string;
  arguments?: Record<string, unknown>;
  resultPreview?: string;
  success?: boolean;
  error?: string | null;
  durationMs?: number;
  streaming?: boolean;
}

// ---------------------------------------------------------------------------
// CascadeNode types
// ---------------------------------------------------------------------------

type NodeType = "input" | "thought" | "action" | "observation" | "response";

interface CascadeNodeData {
  type: NodeType;
  key: string;
  summary: string;
  detail?: string;
  durationMs?: number;
  success?: boolean;
  error?: string | null;
  arguments?: Record<string, unknown>;
  streaming?: boolean;
}

const NODE_CONFIG: Record<
  NodeType,
  { icon: typeof Brain; label: string; color: string; borderColor: string }
> = {
  input: {
    icon: MessageSquare,
    label: "User Input",
    color: "text-amber-500",
    borderColor: "border-amber-500/30",
  },
  thought: {
    icon: Brain,
    label: "Thought",
    color: "text-purple-500",
    borderColor: "border-purple-500/30",
  },
  action: {
    icon: Wrench,
    label: "Action",
    color: "text-blue-500",
    borderColor: "border-blue-500/30",
  },
  observation: {
    icon: Eye,
    label: "Observation",
    color: "text-green-500",
    borderColor: "border-green-500/30",
  },
  response: {
    icon: Bot,
    label: "Response",
    color: "text-muted-foreground",
    borderColor: "border-muted-foreground/30",
  },
};

// ---------------------------------------------------------------------------
// CascadeNode component
// ---------------------------------------------------------------------------

function CascadeNode({ node }: { node: CascadeNodeData }) {
  const [expanded, setExpanded] = useState(false);
  const config = NODE_CONFIG[node.type];
  const Icon = config.icon;

  const hasDetail =
    node.detail || node.arguments || node.type === "observation";
  const isClickable = hasDetail && !node.streaming;

  return (
    <div className={`ml-4 border-l-2 ${config.borderColor} pl-3 py-1`}>
      <div
        className={`flex items-center gap-2 text-xs ${isClickable ? "cursor-pointer" : ""}`}
        onClick={() => isClickable && setExpanded((v) => !v)}
      >
        {/* Expand chevron */}
        {isClickable ? (
          expanded ? (
            <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
          )
        ) : (
          <span className="w-3 shrink-0" />
        )}

        {/* Icon */}
        {node.streaming ? (
          <Loader2 className={`h-3.5 w-3.5 shrink-0 animate-spin ${config.color}`} />
        ) : (
          <Icon className={`h-3.5 w-3.5 shrink-0 ${config.color}`} />
        )}

        {/* Label */}
        <Badge
          variant="outline"
          className={`text-[10px] px-1.5 py-0 ${config.color} border-current/20`}
        >
          {config.label}
        </Badge>

        {/* Summary text */}
        <span className="truncate text-muted-foreground max-w-md">
          {node.summary}
        </span>

        {/* Duration badge */}
        {node.durationMs != null && (
          <Badge variant="secondary" className="text-[10px] px-1 py-0 ml-auto shrink-0">
            {node.durationMs}ms
          </Badge>
        )}

        {/* Success/fail indicator */}
        {node.success != null && (
          <span className={node.success ? "text-green-500" : "text-red-500"}>
            {node.success ? "✓" : "✗"}
          </span>
        )}
      </div>

      {/* Expanded detail */}
      {expanded && (
        <div className="mt-1.5 ml-8 space-y-1.5">
          {node.arguments && (
            <details open className="group">
              <summary className="cursor-pointer text-[10px] text-muted-foreground hover:text-foreground font-medium">
                Arguments
              </summary>
              <pre className="mt-1 max-h-60 overflow-auto rounded-md bg-muted/30 p-2 text-[10px] leading-tight font-mono">
                {JSON.stringify(node.arguments, null, 2)}
              </pre>
            </details>
          )}
          {node.detail && (
            <div className="max-h-60 overflow-auto rounded-md bg-muted/30 p-2">
              <pre className="text-[10px] leading-tight font-mono whitespace-pre-wrap break-words">
                {node.detail}
              </pre>
            </div>
          )}
          {node.error && (
            <div className="text-[10px] text-red-400">Error: {node.error}</div>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Turn → CascadeNode[] conversion
// ---------------------------------------------------------------------------

function turnToNodes(turn: TurnTrace, turnIndex: number): CascadeNodeData[] {
  const nodes: CascadeNodeData[] = [];

  // User Input node
  if (turn.input_text) {
    nodes.push({
      type: "input",
      key: `t${turnIndex}-input`,
      summary: turn.input_text.length > 80
        ? turn.input_text.slice(0, 80) + "..."
        : turn.input_text,
      detail: turn.input_text,
    });
  }

  for (const iter of turn.iterations) {
    const isLastIteration =
      iter.index === turn.iterations[turn.iterations.length - 1]?.index;
    const hasTools = iter.tool_calls.length > 0;

    // Thought node (reasoning text from iterations with tool calls)
    if (hasTools && iter.reasoning_text) {
      nodes.push({
        type: "thought",
        key: `t${turnIndex}-i${iter.index}-thought`,
        summary: iter.text_preview || "(thinking...)",
        detail: iter.reasoning_text,
      });
    }

    // Action + Observation pairs
    for (const tc of iter.tool_calls) {
      nodes.push({
        type: "action",
        key: `t${turnIndex}-i${iter.index}-action-${tc.id}`,
        summary: tc.name,
        arguments: tc.arguments,
        durationMs: tc.duration_ms,
        success: tc.success,
        error: tc.error,
      });
      nodes.push({
        type: "observation",
        key: `t${turnIndex}-i${iter.index}-obs-${tc.id}`,
        summary: tc.success
          ? tc.result_preview.slice(0, 100)
          : `Error: ${tc.error ?? "unknown"}`,
        detail: tc.result_preview,
        success: tc.success,
        error: tc.error,
      });
    }

    // Response node (final iteration text, no tool calls)
    if (isLastIteration && !hasTools && iter.reasoning_text) {
      nodes.push({
        type: "response",
        key: `t${turnIndex}-i${iter.index}-response`,
        summary: iter.text_preview || "(empty response)",
        detail: iter.reasoning_text,
      });
    }
  }

  return nodes;
}

// ---------------------------------------------------------------------------
// TurnGroup component
// ---------------------------------------------------------------------------

function TurnGroup({
  turn,
  turnIndex,
  streamingNodes,
}: {
  turn?: TurnTrace;
  turnIndex: number;
  streamingNodes?: StreamingNode[];
}) {
  const [collapsed, setCollapsed] = useState(false);

  const historicalNodes = turn ? turnToNodes(turn, turnIndex) : [];

  // Convert streaming nodes to CascadeNodeData
  const liveNodes: CascadeNodeData[] = (streamingNodes ?? []).map((sn) => ({
    type: sn.type,
    key: sn.key,
    summary:
      sn.type === "thought"
        ? sn.text?.slice(0, 80) || "(thinking...)"
        : sn.type === "action"
          ? sn.toolName ?? ""
          : sn.resultPreview?.slice(0, 100) ?? "",
    detail: sn.text ?? sn.resultPreview,
    arguments: sn.arguments,
    durationMs: sn.durationMs,
    success: sn.success,
    error: sn.error,
    streaming: sn.streaming,
  }));

  const allNodes = [...historicalNodes, ...liveNodes];

  return (
    <div className="rounded-lg border border-border/50 p-3">
      <div
        className="flex items-center gap-2 cursor-pointer text-xs font-medium"
        onClick={() => setCollapsed((v) => !v)}
      >
        {collapsed ? (
          <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
        ) : (
          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
        )}
        {turn ? (
          <span className={turn.success ? "text-green-500" : "text-red-500"}>
            {turn.success ? "●" : "✗"}
          </span>
        ) : (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-500" />
        )}
        <span>TICK {turnIndex + 1}</span>
        {turn && (
          <span className="text-muted-foreground font-normal">
            {turn.model} · {(turn.duration_ms / 1000).toFixed(1)}s ·{" "}
            {turn.total_tool_calls} tools
          </span>
        )}
        {!turn && (
          <span className="text-blue-500 font-normal animate-pulse">
            running...
          </span>
        )}
      </div>
      {!collapsed && (
        <div className="mt-2">
          {allNodes.map((node) => (
            <CascadeNode key={node.key} node={node} />
          ))}
          {allNodes.length === 0 && (
            <div className="ml-4 text-[10px] text-muted-foreground italic">
              Waiting for events...
            </div>
          )}
        </div>
      )}
      {turn?.error && (
        <div className="ml-4 mt-1 text-xs text-red-500">
          Error: {turn.error}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// CascadeViewer (main export)
// ---------------------------------------------------------------------------

export type { TurnTrace, IterationTrace, ToolCallTrace };

export default function CascadeViewer({
  traces,
  streamingNodes,
  isStreaming,
}: {
  traces: TurnTrace[];
  streamingNodes?: StreamingNode[];
  isStreaming?: boolean;
}) {
  return (
    <div className="space-y-2 font-mono text-xs">
      {traces.map((turn, ti) => (
        <TurnGroup key={ti} turn={turn} turnIndex={ti} />
      ))}
      {/* Active streaming turn (no TurnTrace yet) */}
      {isStreaming && (
        <TurnGroup
          turnIndex={traces.length}
          streamingNodes={streamingNodes}
        />
      )}
      {traces.length === 0 && !isStreaming && (
        <div className="text-muted-foreground italic">
          No turns recorded yet
        </div>
      )}
    </div>
  );
}
```

**Step 2: Verify frontend build**

Run: `cd web && npm run build`
Expected: PASS (no TypeScript errors)

**Step 3: Commit**

```bash
git add web/src/components/CascadeViewer.tsx
git commit -m "feat(web): add CascadeViewer component with ReAct cascade tree"
```

---

## Task 4: Wire CascadeViewer into KernelTop + Streaming

**Files:**
- Modify: `web/src/pages/KernelTop.tsx`

**Step 1: Replace TurnTraceTree with CascadeViewer**

In `KernelTop.tsx`:

1. Update the `TurnTrace` and `IterationTrace` interfaces to include new fields:
```typescript
interface IterationTrace {
  index: number;
  first_token_ms: number | null;
  stream_ms: number;
  text_preview: string;
  reasoning_text: string | null;  // NEW
  tool_calls: ToolCallTrace[];
}

interface TurnTrace {
  duration_ms: number;
  model: string;
  input_text: string | null;  // NEW
  iterations: IterationTrace[];
  final_text_len: number;
  total_tool_calls: number;
  success: boolean;
  error: string | null;
}
```

2. Import CascadeViewer:
```typescript
import CascadeViewer, { type StreamingNode } from "@/components/CascadeViewer";
```

3. Add a `useProcessStream` hook for WebSocket streaming:

```typescript
function useProcessStream(agentId: string | null, processState: string | null) {
  const [nodes, setNodes] = useState<StreamingNode[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);

  useEffect(() => {
    if (!agentId || !processState) return;
    if (processState !== "Running" && processState !== "Idle") return;

    const host = window.location.host;
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const token = localStorage.getItem("access_token") ?? "";
    const ws = new WebSocket(
      `${protocol}//${host}/api/v1/kernel/processes/${agentId}/stream?token=${token}`
    );

    let currentThought = "";
    let thoughtKey = `live-thought-${Date.now()}`;

    ws.onopen = () => {
      setIsStreaming(true);
      setNodes([]);
    };

    ws.onmessage = (ev) => {
      try {
        const event = JSON.parse(ev.data);
        switch (event.type) {
          case "text_delta":
            currentThought += event[0] ?? event.text ?? "";
            setNodes((prev) => {
              const existing = prev.find((n) => n.key === thoughtKey);
              if (existing) {
                return prev.map((n) =>
                  n.key === thoughtKey
                    ? { ...n, text: currentThought }
                    : n
                );
              }
              return [
                ...prev,
                {
                  type: "thought" as const,
                  key: thoughtKey,
                  text: currentThought,
                  streaming: true,
                },
              ];
            });
            break;

          case "tool_call_start":
            // Finalize current thought
            if (currentThought) {
              setNodes((prev) =>
                prev.map((n) =>
                  n.key === thoughtKey ? { ...n, streaming: false } : n
                )
              );
            }
            setNodes((prev) => [
              ...prev,
              {
                type: "action" as const,
                key: `live-action-${event.id}`,
                toolName: event.name,
                toolId: event.id,
                arguments: event.arguments,
                streaming: true,
              },
            ]);
            break;

          case "tool_call_end":
            setNodes((prev) => [
              ...prev.map((n) =>
                n.key === `live-action-${event.id}`
                  ? { ...n, streaming: false, success: event.success }
                  : n
              ),
              {
                type: "observation" as const,
                key: `live-obs-${event.id}`,
                resultPreview: event.result_preview,
                success: event.success,
                error: event.error,
              },
            ]);
            break;

          case "turn_metrics":
            // Turn completed — reset for next turn
            currentThought = "";
            thoughtKey = `live-thought-${Date.now()}`;
            break;

          case "done":
            setIsStreaming(false);
            setNodes([]);
            break;
        }
      } catch {
        // ignore parse errors
      }
    };

    ws.onclose = () => {
      setIsStreaming(false);
    };

    ws.onerror = () => {
      setIsStreaming(false);
    };

    return () => {
      ws.close();
      setIsStreaming(false);
      setNodes([]);
    };
  }, [agentId, processState]);

  return { streamingNodes: nodes, isStreaming };
}
```

4. In the `KernelTop` component, find the selected process state:
```typescript
const selectedProcessState = processes.find(
  (p) => p.agent_id === selectedProcess
)?.state ?? null;

const { streamingNodes, isStreaming } = useProcessStream(
  selectedProcess,
  selectedProcessState
);
```

5. Replace the `<TurnTraceTree>` usage (around line 561) with:
```tsx
<CascadeViewer
  traces={turnsQuery.data ?? []}
  streamingNodes={streamingNodes}
  isStreaming={isStreaming}
/>
```

6. Remove the old `ToolCallDetail` and `TurnTraceTree` functions (lines 171-270).

7. Add `useEffect` import if not present.

**Step 2: Verify frontend build**

Run: `cd web && npm run build`
Expected: PASS

**Step 3: Commit**

```bash
git add web/src/pages/KernelTop.tsx
git commit -m "feat(web): wire CascadeViewer into KernelTop with streaming support"
```

---

## Task 5: Handle TextDelta serialization format

**Files:**
- Verify: `crates/core/kernel/src/io/stream.rs` line 71

**Step 1: Check StreamEvent::TextDelta serialization**

The `TextDelta(String)` variant serializes differently than a struct. With `#[serde(tag = "type")]`, it becomes `{"type": "text_delta", "0": "text"}` — a tuple variant.

However, the frontend Chat.tsx already handles this (it works). Check the actual format by looking at how WebAdapter converts StreamEvent to WebEvent. The WebAdapter may already do its own conversion.

Verify: Read `crates/extensions/channels/src/web.rs` around the stream forwarder code to confirm the JSON format the frontend receives.

If the format differs between the kernel `StreamEvent` and what the new WS endpoint sends directly, adjust the frontend parsing in `useProcessStream`.

**Step 2: Fix TextDelta parsing if needed**

If `StreamEvent::TextDelta` serializes as `{"type":"text_delta","0":"hello"}`, update the frontend:
```typescript
case "text_delta":
  currentThought += event["0"] ?? "";
```

Or rename the variant to use a struct:
```rust
TextDelta { text: String },
```

Choose whichever is simpler. Likely the frontend parse fix is sufficient.

**Step 3: Commit if changes made**

---

## Task 6: Full Integration Test

**Step 1: Build backend**

Run: `cargo check` (full workspace)
Expected: PASS

**Step 2: Build frontend**

Run: `cd web && npm run build`
Expected: PASS

**Step 3: Run kernel tests**

Run: `cargo test -p rara-kernel`
Expected: All tests pass

**Step 4: Run backend-admin tests**

Run: `cargo test -p rara-backend-admin`
Expected: All tests pass

**Step 5: Final commit (if any remaining fixes)**

```bash
git commit -m "fix: cascade viewer integration fixes"
```
