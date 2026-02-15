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

export interface TaskRunRecord {
  id: string;
  task_id: string;
  status: string;
  started_at: string;
  finished_at: string | null;
  error_message: string | null;
}

// Saved Jobs
export interface SavedJob {
  id: string;
  url: string;
  title: string | null;
  company: string | null;
  status: string;
  markdown_s3_key: string | null;
  analysis_result: Record<string, unknown> | null;
  match_score: number | null;
  error_message: string | null;
  crawled_at: string | null;
  analyzed_at: string | null;
  expires_at: string | null;
  created_at: string;
  updated_at: string;
}

// Pipeline Events
export interface PipelineEvent {
  id: string;
  saved_job_id: string;
  stage: string;
  event_kind: string;
  message: string;
  metadata: Record<string, unknown> | null;
  created_at: string;
}

export const PIPELINE_STAGES = ["crawl", "analyze", "gc"] as const;
export type PipelineStage = (typeof PIPELINE_STAGES)[number];

export const PIPELINE_EVENT_KINDS = [
  "started",
  "completed",
  "failed",
  "info",
] as const;
export type PipelineEventKind = (typeof PIPELINE_EVENT_KINDS)[number];

export const SAVED_JOB_STATUSES = [
  "pending_crawl",
  "crawling",
  "crawled",
  "analyzing",
  "analyzed",
  "failed",
  "expired",
] as const;

export type SavedJobStatus = (typeof SAVED_JOB_STATUSES)[number];

// Runtime Settings
export interface RuntimeSettingsView {
  ai: {
    configured: boolean;
    default_model: string | null;
    job_model: string | null;
    chat_model: string | null;
    openrouter_api_key: string | null;
  };
  telegram: {
    configured: boolean;
    chat_id: number | null;
    allowed_group_chat_id: number | null;
    token_hint: string | null;
  };
  agent: {
    soul: string | null;
    chat_system_prompt: string | null;
  };
  updated_at: string | null;
}

export interface RuntimeSettingsPatch {
  ai?: {
    openrouter_api_key?: string;
    default_model?: string;
    job_model?: string | null;
    chat_model?: string | null;
  };
  telegram?: {
    bot_token?: string;
    chat_id?: number;
    allowed_group_chat_id?: number;
  };
  agent?: {
    soul?: string | null;
    chat_system_prompt?: string | null;
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
