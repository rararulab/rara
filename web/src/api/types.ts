/*
 * Copyright 2025 Crrow
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

// Job Discovery
export interface DiscoveryCriteria {
  keywords: string[];
  location?: string;
  job_type?: string;
  max_results?: number;
  sites?: string[];
  posted_after?: string;
}

export interface NormalizedJob {
  id: string;
  source_job_id: string;
  source_name: string;
  title: string;
  company: string;
  location?: string;
  description?: string;
  url?: string;
  salary_min?: number;
  salary_max?: number;
  salary_currency?: string;
  tags: string[];
  posted_at?: string;
  job_type?: string;
  is_remote?: boolean;
  salary_interval?: string;
  salary_source?: string;
  job_level?: string;
  company_url?: string;
  company_industry?: string;
}

// Analytics
export interface MetricsSnapshot {
  id: string;
  period: string;
  total_applications: number;
  total_interviews: number;
  total_offers: number;
  total_rejections: number;
  total_ai_runs: number;
  total_ai_cost_cents: number;
  snapshot_date: string;
  created_at: string;
}

export interface DerivedRates {
  offer_rate: number;
  interview_rate: number;
  rejection_rate: number;
  avg_ai_cost_per_run: number;
}

// Applications
export interface Application {
  id: string;
  company_name: string;
  position_title: string;
  status: string;
  job_url: string | null;
  notes: string | null;
  applied_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface StatusChangeRecord {
  id: string;
  application_id: string;
  from_status: string | null;
  to_status: string;
  source: string | null;
  note: string | null;
  changed_at: string;
}

export const APPLICATION_STATUSES = [
  "draft",
  "applied",
  "screening",
  "interviewing",
  "offer",
  "accepted",
  "rejected",
  "withdrawn",
] as const;

export type ApplicationStatus = (typeof APPLICATION_STATUSES)[number];

// Resumes
export interface Resume {
  id: string;
  title: string;
  target_role: string | null;
  content: string | null;
  version: number;
  created_at: string;
  updated_at: string;
}

// Interviews
export interface InterviewPlan {
  id: string;
  application_id: string | null;
  company_name: string;
  position_title: string;
  status: string;
  prep_materials: string | null;
  interview_date: string | null;
  created_at: string;
  updated_at: string;
}

// Notifications (queue observability)
export type QueueMessageState = "ready" | "inflight" | "archived";

export interface NotificationQueueOverview {
  queue_name: string;
  ready_count: number;
  inflight_count: number;
  archived_count: number;
}

export interface NotificationQueueMessage {
  state: QueueMessageState;
  msg_id: number;
  read_ct: number;
  enqueued_at: string;
  vt: string;
  archived_at: string | null;
  payload: {
    id?: string;
    chat_id?: number | null;
    subject?: string | null;
    body?: string | null;
    [key: string]: unknown;
  };
}

export interface NotificationQueueMessagesResponse {
  state: QueueMessageState;
  limit: number;
  offset: number;
  items: NotificationQueueMessage[];
}

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

// Agent Scheduler (jobs created by the AI agent)
export interface AgentJob {
  id: string;
  message: string;
  trigger: AgentTrigger;
  session_key: string;
  created_at: string;
  last_run_at: string | null;
  enabled: boolean;
}

export type AgentTrigger =
  | { type: "cron"; expr: string }
  | { type: "delay"; run_at: string }
  | { type: "interval"; seconds: number };

export interface TaskRunRecord {
  id: string;
  task_id: string;
  status: string;
  started_at: string;
  finished_at: string | null;
  error_message: string | null;
}

// Runtime Settings
export interface RuntimeSettingsView {
  ai: {
    configured: boolean;
    default_model: string | null;
    job_model: string | null;
    chat_model: string | null;
    chat_model_fallbacks: string[];
    job_model_fallbacks: string[];
    openrouter_api_key: string | null;
  };
  telegram: {
    configured: boolean;
    chat_id: number | null;
    allowed_group_chat_id: number | null;
    notification_channel_id: number | null;
    token_hint: string | null;
  };
  agent: {
    soul: string | null;
    chat_system_prompt: string | null;
    memory: {
      chroma_url: string | null;
      chroma_collection: string | null;
      chroma_api_key_hint: string | null;
    };
    composio: {
      api_key: string | null;
      entity_id: string | null;
    };
  };
  job_pipeline?: JobPipelineSettings;
  gmail?: GmailSettings;
  updated_at: string | null;
}

export interface RuntimeSettingsPatch {
  ai?: {
    openrouter_api_key?: string;
    default_model?: string;
    job_model?: string | null;
    chat_model?: string | null;
    chat_model_fallbacks?: string[];
    job_model_fallbacks?: string[];
  };
  telegram?: {
    bot_token?: string;
    chat_id?: number;
    allowed_group_chat_id?: number;
    notification_channel_id?: number | null;
  };
  agent?: {
    soul?: string | null;
    chat_system_prompt?: string | null;
    memory?: {
      chroma_url?: string;
      chroma_collection?: string;
      chroma_api_key?: string;
    };
    composio?: {
      api_key?: string;
      entity_id?: string;
    };
  };
}

export interface PromptFileView {
  name: string;
  description: string;
  content: string;
}

export interface PromptListView {
  prompts: PromptFileView[];
}

// Chat Models
export interface ChatModel {
  id: string;
  name: string;
  context_length: number;
  is_favorite: boolean;
}

// Typst
export interface TypstProject {
  id: string;
  name: string;
  local_path: string;
  main_file: string;
  git_url: string | null;
  git_last_synced_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface FileEntry {
  path: string;
  is_dir: boolean;
  children?: FileEntry[];
}

export interface FileContent {
  path: string;
  content: string;
}

export interface RenderResult {
  id: string;
  project_id: string;
  pdf_object_key: string;
  source_hash: string;
  page_count: number;
  file_size: number;
  created_at: string;
}

export interface JustRecipe {
  name: string;
  description: string | null;
}

export interface RunOutput {
  exit_code: number;
  stdout: string;
  stderr: string;
}

// System / Directory Browser
export interface BrowseResult {
  current_path: string;
  parent_path: string | null;
  entries: BrowseDirEntry[];
}

export interface BrowseDirEntry {
  name: string;
  path: string;
  has_typ_files: boolean;
}

// Chat Sessions
export interface ChatSession {
  key: string;
  title: string | null;
  model: string | null;
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
  | { type: "image_url"; url: string };

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

// Telegram Contacts
export interface TelegramContact {
  id: string;
  name: string;
  telegram_username: string;
  chat_id: number | null;
  notes: string | null;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateContactRequest {
  name: string;
  telegram_username: string;
  notes?: string;
  enabled?: boolean;
}

export interface UpdateContactRequest {
  name?: string;
  telegram_username?: string;
  notes?: string | null;
  enabled?: boolean;
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

// Job Pipeline Settings
export interface JobPipelineSettings {
  job_preferences: string | null;
  score_threshold_auto: number;
  score_threshold_notify: number;
  resume_project_path: string | null;
}

// Gmail Settings
export interface GmailSettings {
  configured: boolean;
  auto_send_enabled: boolean;
  address: string | null;
  app_password_hint: string | null;
}

// Pipeline Status
export interface PipelineStatus {
  running: boolean;
}

// Pipeline Run (from backend)
export interface PipelineRun {
  id: string;
  status: "Running" | "Completed" | "Failed" | "Cancelled";
  started_at: string;
  finished_at: string | null;
  jobs_found: number;
  jobs_scored: number;
  jobs_applied: number;
  jobs_notified: number;
  summary: string | null;
  error: string | null;
}

// Pipeline Discovered Job (with details from job table JOIN)
export interface PipelineDiscoveredJob {
  id: string;
  run_id: string;
  job_id: string;
  score: number | null;
  action: "Discovered" | "Notified" | "Applied" | "Skipped";
  created_at: string;
  // Job details from JOIN
  title: string;
  company: string;
  location: string | null;
  url: string | null;
  description: string | null;
  posted_at: string | null;
}

// Discovered Jobs Stats
export interface DiscoveredJobsStats {
  total: number;
  by_action: {
    discovered: number;
    notified: number;
    applied: number;
    skipped: number;
  };
  scored_count: number;
  avg_score: number | null;
}

export interface PaginatedDiscoveredJobs {
  items: PipelineDiscoveredJob[];
  total: number;
  limit: number;
  offset: number;
}

// Pipeline Run Event (stored in DB)
export interface PipelineRunEvent {
  id: number;
  run_id: string;
  seq: number;
  event_type: string;
  payload: Record<string, unknown>;
  created_at: string;
}

// Pipeline Stream Event (SSE)
export type PipelineStreamEvent =
  | { type: "started"; run_id: string }
  | { type: "iteration"; index: number }
  | { type: "thinking" }
  | { type: "thinking_done" }
  | { type: "tool_call_start"; id: string; name: string; arguments?: unknown }
  | { type: "tool_call_end"; id: string; name: string; success: boolean; error?: string; result?: unknown }
  | { type: "text_delta"; text: string }
  | { type: "done"; summary: string; iterations: number; tool_calls: number }
  | { type: "error"; message: string };

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
