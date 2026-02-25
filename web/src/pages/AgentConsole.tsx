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
import { Activity, Bot, Clock, Server, Terminal, Wrench } from "lucide-react";
import { TabBar } from "@/components/TabBar";
import type { Tab } from "@/components/TabBar";
import Chat from "@/pages/Chat";
import AgentStatus from "@/pages/AgentStatus";
import Skills from "@/pages/Skills";
import { AgentJobsPanel } from "@/pages/Scheduler";
import McpServers from "@/pages/McpServers";
import AgentDispatcher from "@/pages/AgentDispatcher";
import CodingTasks from "@/pages/CodingTasks";

const AGENT_TABS: Tab[] = [
  { key: "chat", label: "Chat", icon: <Bot className="h-4 w-4" /> },
  { key: "status", label: "Status", icon: <Activity className="h-4 w-4" /> },
  { key: "skills", label: "Skills", icon: <Wrench className="h-4 w-4" /> },
  { key: "tasks", label: "Tasks", icon: <Terminal className="h-4 w-4" /> },
  {
    key: "scheduler",
    label: "Scheduler",
    icon: <Clock className="h-4 w-4" />,
  },
  {
    key: "dispatcher",
    label: "Dispatcher",
    icon: <Activity className="h-4 w-4" />,
  },
  {
    key: "mcp",
    label: "MCP",
    icon: <Server className="h-4 w-4" />,
  },
];

export default function AgentConsole() {
  const [searchParams, setSearchParams] = useSearchParams();
  const activeTab = searchParams.get("tab") ?? "chat";

  const setTab = (tab: string) => setSearchParams({ tab });

  const isChatTab = activeTab === "chat";

  return (
    <div className="flex h-full flex-col">
      <TabBar tabs={AGENT_TABS} activeTab={activeTab} onTabChange={setTab} />
      <div
        className={
          isChatTab
            ? "flex flex-1 min-h-0 flex-col"
            : "flex-1 min-h-0 overflow-auto"
        }
      >
        {activeTab === "chat" && <Chat />}
        {activeTab === "status" && <AgentStatus />}
        {activeTab === "skills" && (
          <div className="p-6">
            <Skills />
          </div>
        )}
        {activeTab === "tasks" && (
          <div className="p-6">
            <CodingTasks />
          </div>
        )}
        {activeTab === "scheduler" && (
          <div className="p-6">
            <AgentJobsPanel />
          </div>
        )}
        {activeTab === "dispatcher" && (
          <div className="p-6">
            <AgentDispatcher />
          </div>
        )}
        {activeTab === "mcp" && (
          <div className="p-6">
            <McpServers />
          </div>
        )}
      </div>
    </div>
  );
}
