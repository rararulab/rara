export interface Resume {
  id: string;
  title: string;
  target_role: string | null;
  content: string | null;
  version: number;
  created_at: string;
  updated_at: string;
}

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
