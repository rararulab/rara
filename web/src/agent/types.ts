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

/**
 * Local re-export surface for the `RaraAgent` ↔ `<pi-chat-panel>` /
 * `<agent-interface>` contract.
 *
 * Phase (c) of #1935 introduces these types as a thin shim over pi-ai +
 * pi-web-ui's existing exports. Once phase (d) wires `PiChat.tsx` against
 * `RaraAgent` and drops `@mariozechner/pi-agent-core`, these become the
 * single source of truth — callers import `AgentEvent`, `AgentTool`,
 * `AgentMessage`, `ThinkingLevel`, `RaraAgentState` from here, never from
 * pi-agent-core.
 *
 * Today (phase c) the structural definitions are kept assignable to
 * pi-agent-core's equivalents so a partial migration is a one-line import
 * swap rather than a deep refactor.
 */

import type {
  Api,
  AssistantMessage,
  AssistantMessageEvent,
  ImageContent,
  Message,
  Model,
  TextContent,
  Tool,
  ToolResultMessage,
} from '@mariozechner/pi-ai';
import type { ArtifactMessage, UserMessageWithAttachments } from '@mariozechner/pi-web-ui';
import type { Static, TSchema } from '@sinclair/typebox';

/**
 * Reasoning/thinking level supported by the chat panel selector.
 *
 * Mirrors `pi-agent-core`'s `ThinkingLevel`; `xhigh` is preserved for
 * source-compat with pi-ai's gpt-5.x families even though rara may not
 * surface it in the UI today.
 */
export type ThinkingLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh';

/**
 * Result returned from an `AgentTool.execute` call.
 *
 * Identical shape to pi-agent-core's `AgentToolResult` so messages built
 * by the renderer remain assignable in either direction during the
 * phase (c) → phase (d) transition.
 */
export interface AgentToolResult<TDetails = unknown> {
  content: (TextContent | ImageContent)[];
  details: TDetails;
}

/**
 * Tool descriptor consumed by `<pi-chat-panel>` for renderer decoration
 * (chip card titles, parameter previews). The kernel runs the actual
 * tool; `RaraAgent` never invokes `execute` itself, so most callers
 * supply a stub. Kept structurally compatible with pi-agent-core's
 * `AgentTool<TParameters, TDetails>`.
 */
export interface AgentTool<
  TParameters extends TSchema = TSchema,
  TDetails = unknown,
> extends Tool<TParameters> {
  label: string;
  execute: (
    toolCallId: string,
    params: Static<TParameters>,
    signal?: AbortSignal,
  ) => Promise<AgentToolResult<TDetails>>;
}

/**
 * Union of pi-ai's message types plus pi-web-ui's custom variants
 * (`user-with-attachments`, `artifact`). Mirrors the declaration-merged
 * `AgentMessage` from pi-agent-core (`CustomAgentMessages`).
 */
export type AgentMessage = Message | UserMessageWithAttachments | ArtifactMessage;

/**
 * Live mutable state surface read by `<pi-chat-panel>` and
 * `<agent-interface>` (see `AgentInterface.ts:179` for the assignments
 * the host performs).
 *
 * RaraAgent exposes the same object reference on every `state` access so
 * `state.model = …` mutations from `PiChat.tsx` land on the agent itself.
 */
export interface RaraAgentState {
  /** Reserved for parity with pi-agent-core; rara serves the system prompt server-side. */
  systemPrompt: string;
  /** Currently selected LLM model — read by `<message-editor>` for the picker. */
  model: Model<Api>;
  /** Reasoning level — read by the thinking selector. */
  thinkingLevel: ThinkingLevel;
  /**
   * Tool descriptors used purely for renderer decoration. The kernel
   * executes tools; RaraAgent never calls `execute()`. This array is
   * mirrored from `agent.setTools(tools)` calls.
   */
  tools: AgentTool[];
  /** Conversation history — owned by RaraAgent, mutated in place as frames arrive. */
  messages: AgentMessage[];
  /** True between the first frame of a turn and the matching `done`/`error`. */
  isStreaming: boolean;
  /** The currently-streaming assistant message, or `null` between turns. */
  streamMessage: AssistantMessage | null;
  /** Tool-call ids whose `tool_call_end` frame has not yet arrived. */
  pendingToolCalls: Set<string>;
  /** Last error message surfaced to the UI; cleared on the next `agent_start`. */
  error: string | undefined;
}

/**
 * Lifecycle events emitted by `RaraAgent.subscribe(...)`. Field shapes
 * match the seven `AgentEvent` variants `<agent-interface>` reads at
 * `AgentInterface.ts:153-186`. The two `tool_execution_*` variants from
 * pi-agent-core are intentionally NOT emitted — rara executes tools in
 * the kernel, the UI layer learns about them via `tool_call_start` /
 * `tool_call_end` frames folded into `message_update`.
 */
export type RaraAgentEvent =
  | { type: 'agent_start' }
  | { type: 'agent_end'; messages: AgentMessage[] }
  | { type: 'turn_start' }
  | { type: 'turn_end'; message: AgentMessage; toolResults: ToolResultMessage[] }
  | { type: 'message_start'; message: AgentMessage }
  | {
      type: 'message_update';
      message: AgentMessage;
      assistantMessageEvent: AssistantMessageEvent;
    }
  | { type: 'message_end'; message: AgentMessage };
