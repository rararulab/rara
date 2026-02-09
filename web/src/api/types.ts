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
