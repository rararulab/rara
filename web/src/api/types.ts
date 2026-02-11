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

// Notifications
export interface Notification {
  id: string;
  channel: string;
  status: string;
  subject: string | null;
  body: string;
  error_message: string | null;
  created_at: string;
  sent_at: string | null;
}

export interface NotificationStatistics {
  total: number;
  pending: number;
  sent: number;
  failed: number;
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
  markdown_preview: string | null;
  analysis_result: Record<string, unknown> | null;
  match_score: number | null;
  error_message: string | null;
  crawled_at: string | null;
  analyzed_at: string | null;
  expires_at: string | null;
  created_at: string;
  updated_at: string;
}

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
    model: string | null;
    key_hint: string | null;
  };
  telegram: {
    configured: boolean;
    chat_id: number | null;
    token_hint: string | null;
  };
}

export interface RuntimeSettingsPatch {
  ai?: {
    openrouter_api_key?: string;
    model?: string;
  };
  telegram?: {
    bot_token?: string;
    chat_id?: number;
  };
}
