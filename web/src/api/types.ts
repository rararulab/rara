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

// Scheduler
export interface ScheduledTask {
  id: string;
  name: string;
  cron_expression: string;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
  created_at: string;
}

export interface TaskRunRecord {
  id: string;
  task_id: string;
  status: string;
  started_at: string;
  finished_at: string | null;
  error_message: string | null;
}

// Flat KV Settings
export type SettingsMap = Record<string, string>;
export interface SettingValue { key: string; value: string; }
export type SettingsPatch = Record<string, string | null>;

// Chat Models
export interface ChatModel {
  id: string;
  name: string;
  context_length: number;
  is_favorite: boolean;
}


// Chat Sessions

/** LLM thinking-level override persisted per session. Mirrors pi-mono's
 *  six-bucket scale so the chat-panel selector round-trips without any
 *  lossy mapping. */
export type ThinkingLevel =
  | "off"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh";

export interface ChatSession {
  key: string;
  title: string | null;
  model: string | null;
  model_provider: string | null;
  thinking_level: ThinkingLevel | null;
  system_prompt: string | null;
  message_count: number;
  preview: string | null;
  metadata: Record<string, unknown> | null;
  created_at: string;
  updated_at: string;
}

export interface ChatMessageData {
  seq: number;
  role: "system" | "user" | "assistant" | "tool" | "tool_result";
  content: string | ChatContentBlock[];
  tool_call_id?: string;
  tool_name?: string;
  created_at: string;
}

export type ChatContentBlock =
  | { type: "text"; text: string }
  | { type: "image_url"; url: string }
  | { type: "image_base64"; media_type: string; data: string }
  | { type: "audio_base64"; media_type: string; data: string };

export interface SendMessageResponse {
  message: ChatMessageData;
}

// SSE stream event types (matches backend ChatStreamEvent)
export type ChatStreamEvent =
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "thinking" }
  | { type: "thinking_done" }
  | { type: "iteration"; index: number }
  | { type: "tool_call_start"; id: string; name: string }
  | { type: "tool_call_end"; id: string; name: string; success: boolean; error?: string }
  | { type: "done"; text: string }
  | { type: "error"; message: string };

// Skills
export interface SkillSummary {
  name: string;
  description: string;
  allowed_tools: string[];
  source: string | null;
  homepage: string | null;
  license: string | null;
  eligible: boolean;
}

export interface SkillDetail extends SkillSummary {
  body: string;
}

export interface CreateSkillRequest {
  name: string;
  description: string;
  allowed_tools: string[];
  prompt: string;
}


// ── MCP Management ──────────────────────────────────────────

export interface McpServerInfo {
  name: string;
  config: McpServerConfig;
  status: McpServerStatus;
}

export interface McpServerConfig {
  command: string;
  args: string[];
  env: Record<string, string>;
  enabled: boolean;
  transport: string;
  url: string | null;
  startup_timeout_secs: number | null;
  tool_timeout_secs: number | null;
  tools_enabled: string[] | null;
  tools_disabled: string[];
}

export type McpServerStatus =
  | { type: 'connected' }
  | { type: 'connecting' }
  | { type: 'disconnected' }
  | { type: 'error'; message: string };

export interface McpToolView {
  name: string;
  description: string | null;
  input_schema: Record<string, unknown>;
}

export interface McpResourceView {
  uri: string;
  name: string | null;
  description: string | null;
  mime_type: string | null;
}

export interface McpLogEntry {
  timestamp: string;
  level: string;
  message: string;
}

export interface CreateMcpServerRequest {
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  enabled: boolean;
  transport: string;
  url?: string;
  startup_timeout_secs?: number;
  tool_timeout_secs?: number;
}


// Agents
export interface AgentResponse {
  name: string;
  description: string;
  model: string | null;
  role: string | null;
  provider_hint: string | null;
  max_iterations: number | null;
  tools: string[];
  builtin: boolean;
}

export interface CreateAgentRequest {
  name: string;
  description: string;
  model: string;
  system_prompt: string;
  soul_prompt?: string;
  provider_hint?: string;
  max_iterations?: number;
  tools?: string[];
}

// Coding Tasks
export type CodingTaskStatus = 'Pending' | 'Cloning' | 'Running' | 'Completed' | 'Failed' | 'Merged' | 'MergeFailed';
export type AgentType = 'Codex' | 'Claude';

export interface CodingTaskSummary {
  id: string;
  status: CodingTaskStatus;
  agent_type: AgentType;
  branch: string;
  prompt: string;
  pr_url: string | null;
  created_at: string;
}

export interface CodingTaskDetail {
  id: string;
  status: CodingTaskStatus;
  agent_type: AgentType;
  repo_url: string;
  branch: string;
  prompt: string;
  pr_url: string | null;
  pr_number: number | null;
  session_key: string | null;
  tmux_session: string;
  workspace_path: string;
  output: string;
  exit_code: number | null;
  error: string | null;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
}

export interface CreateCodingTaskRequest {
  prompt: string;
  agent_type?: AgentType;
  repo_url?: string;
  session_key?: string;
}

