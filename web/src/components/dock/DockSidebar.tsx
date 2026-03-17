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

import { MessageSquare, BookOpen, Clock } from "lucide-react";
import { cn } from "@/lib/utils";
import type { DockStore } from "@/hooks/use-dock-store";
import DockAnnotations from "./DockAnnotations";
import DockFacts from "./DockFacts";
import DockTimeline from "./DockTimeline";

interface DockSidebarProps {
  store: DockStore;
}

const tabs = [
  { key: "annotations" as const, label: "Notes", icon: MessageSquare },
  { key: "facts" as const, label: "Facts", icon: BookOpen },
  { key: "history" as const, label: "History", icon: Clock },
] as const;

export default function DockSidebar({ store }: DockSidebarProps) {
  return (
    <div className="flex h-full w-72 flex-col border-l border-border/40 bg-background/30">
      {/* Tab bar */}
      <div className="flex shrink-0 border-b border-border/40">
        {tabs.map((tab) => {
          const isActive = store.activeTab === tab.key;
          const Icon = tab.icon;
          return (
            <button
              key={tab.key}
              className={cn(
                "flex flex-1 items-center justify-center gap-1.5 px-3 py-2.5 text-xs font-medium transition-colors",
                isActive
                  ? "border-b-2 border-primary text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={() => store.setActiveTab(tab.key)}
            >
              <Icon className="h-3.5 w-3.5" />
              {tab.label}
            </button>
          );
        })}
      </div>

      {/* Tab content */}
      {store.activeTab === "annotations" && <DockAnnotations store={store} />}
      {store.activeTab === "facts" && <DockFacts store={store} />}
      {store.activeTab === "history" && <DockTimeline store={store} />}
    </div>
  );
}
