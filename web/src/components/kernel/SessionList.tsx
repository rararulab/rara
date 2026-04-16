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

import { Zap } from "lucide-react";
import { sessionGroup } from "@/api/kernel-types";
import { SessionListItem } from "./SessionListItem";

export interface SessionEntry {
  agent_id: string;
  manifest_name: string;
  state: string;
  last_activity: string | null;
}

export interface SessionListProps {
  sessions: SessionEntry[];
  selectedId: string | null;
  onSelect: (agentId: string) => void;
  isLoading: boolean;
}

/**
 * Left column: filterable session list grouped as Active / Dormant.
 *
 * Active = sessions whose state is `Active` or `Ready`.
 * Dormant = `Suspended` or `Paused`.
 */
export function SessionList({
  sessions,
  selectedId,
  onSelect,
  isLoading,
}: SessionListProps) {
  const active = sessions.filter((s) => sessionGroup(s.state) === "active");
  const dormant = sessions.filter((s) => sessionGroup(s.state) === "dormant");

  if (isLoading) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
        <Zap className="h-6 w-6 animate-pulse opacity-30" />
        <p className="text-xs">Loading sessions...</p>
      </div>
    );
  }

  if (sessions.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
        <Zap className="h-6 w-6 opacity-20" />
        <p className="text-xs">No sessions</p>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {active.length > 0 && (
        <>
          <div className="sticky top-0 z-[1] bg-background/90 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground backdrop-blur-sm">
            Active ({active.length})
          </div>
          {active.map((s) => (
            <SessionListItem
              key={s.agent_id}
              manifestName={s.manifest_name}
              agentId={s.agent_id}
              state={s.state}
              lastActivity={s.last_activity}
              isSelected={selectedId === s.agent_id}
              onClick={() => onSelect(s.agent_id)}
            />
          ))}
        </>
      )}

      {dormant.length > 0 && (
        <>
          <div className="sticky top-0 z-[1] bg-background/90 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground backdrop-blur-sm">
            Dormant ({dormant.length})
          </div>
          {dormant.map((s) => (
            <SessionListItem
              key={s.agent_id}
              manifestName={s.manifest_name}
              agentId={s.agent_id}
              state={s.state}
              lastActivity={s.last_activity}
              isSelected={selectedId === s.agent_id}
              onClick={() => onSelect(s.agent_id)}
            />
          ))}
        </>
      )}
    </div>
  );
}
