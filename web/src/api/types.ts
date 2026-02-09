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
