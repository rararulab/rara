/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

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
            {node.success ? "\u2713" : "\u2717"}
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
// Turn to CascadeNode[] conversion
// ---------------------------------------------------------------------------

function turnToNodes(turn: TurnTrace, turnIndex: number): CascadeNodeData[] {
  const nodes: CascadeNodeData[] = [];

  // User Input node
  if (turn.input_text) {
    nodes.push({
      type: "input",
      key: `t${turnIndex}-input`,
      summary:
        turn.input_text.length > 80
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
        ? (sn.text?.slice(0, 80) || "(thinking...)")
        : sn.type === "action"
          ? (sn.toolName ?? "")
          : (sn.resultPreview?.slice(0, 100) ?? ""),
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
            {turn.success ? "\u25CF" : "\u2717"}
          </span>
        ) : (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-500" />
        )}
        <span>TICK {turnIndex + 1}</span>
        {turn && (
          <span className="text-muted-foreground font-normal">
            {turn.model} &middot; {(turn.duration_ms / 1000).toFixed(1)}s &middot;{" "}
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
