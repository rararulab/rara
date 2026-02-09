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
