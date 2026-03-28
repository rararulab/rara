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

// ---------------------------------------------------------------------------
// WebSocket event types (matches Rust WebEvent enum)
// ---------------------------------------------------------------------------

export type WebEvent =
  | { type: "message"; content: string }
  | { type: "typing" }
  | { type: "phase"; phase: string }
  | { type: "error"; message: string }
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "tool_call_start"; name: string; id: string; arguments: Record<string, unknown> }
  | { type: "tool_call_end"; id: string; result_preview: string; success: boolean; error: string | null }
  | { type: "progress"; stage: string }
  | { type: "done" }
  | { type: "turn_rationale"; text: string }
  | { type: "turn_metrics"; duration_ms: number; iterations: number; tool_calls: number; model: string };

export interface TurnMetrics {
  duration_ms: number;
  iterations: number;
  tool_calls: number;
  model: string;
}

export interface ActiveToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface CompletedTool {
  id: string;
  name: string;
  success: boolean;
  result_preview: string;
  error: string | null;
}

export interface StreamState {
  isStreaming: boolean;
  text: string;
  reasoning: string;
  isThinking: boolean;
  activeTools: ActiveToolCall[];
  completedTools: CompletedTool[];
  turnRationale: string;
  error: string | null;
}

export type PendingDraft = {
  text: string;
};
