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

import { useSearchParams } from "react-router";
import {
  Briefcase,
  FileText,
  LayoutDashboard,
  MessageSquare,
  Search,
} from "lucide-react";
import { TabBar } from "@/components/TabBar";
import type { Tab } from "@/components/TabBar";
import JobDiscovery from "@/pages/JobDiscovery";
import Applications from "@/pages/Applications";
import Resumes from "@/pages/Resumes";
import Interviews from "@/pages/Interviews";
import Dashboard from "@/pages/Dashboard";

const JOBS_TABS: Tab[] = [
  {
    key: "discovery",
    label: "Discovery",
    icon: <Search className="h-4 w-4" />,
  },
  {
    key: "applications",
    label: "Applications",
    icon: <Briefcase className="h-4 w-4" />,
  },
  { key: "resumes", label: "Resumes", icon: <FileText className="h-4 w-4" /> },
  {
    key: "interviews",
    label: "Interviews",
    icon: <MessageSquare className="h-4 w-4" />,
  },
  {
    key: "dashboard",
    label: "Dashboard",
    icon: <LayoutDashboard className="h-4 w-4" />,
  },
];

export default function JobsWorkspace() {
  const [searchParams, setSearchParams] = useSearchParams();
  const activeTab = searchParams.get("tab") ?? "discovery";

  const setTab = (tab: string) => setSearchParams({ tab });

  return (
    <div className="flex h-full flex-col">
      <TabBar tabs={JOBS_TABS} activeTab={activeTab} onTabChange={setTab} />
      <div className="flex-1 min-h-0 overflow-auto">
        <div className="p-6">
          {activeTab === "discovery" && <JobDiscovery />}
          {activeTab === "applications" && <Applications />}
          {activeTab === "resumes" && <Resumes />}
          {activeTab === "interviews" && <Interviews />}
          {activeTab === "dashboard" && <Dashboard />}
        </div>
      </div>
    </div>
  );
}
