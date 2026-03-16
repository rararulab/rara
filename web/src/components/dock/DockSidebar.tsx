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

export default function DockSidebar() {
  return (
    <div className="flex h-full w-72 flex-col border-l border-border/40 bg-background/30">
      {/* Tab bar */}
      <div className="flex shrink-0 border-b border-border/40">
        <button className="flex flex-1 items-center justify-center gap-1.5 border-b-2 border-primary px-3 py-2.5 text-xs font-medium text-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          Annotations
        </button>
        <button className="flex flex-1 items-center justify-center gap-1.5 px-3 py-2.5 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors">
          <BookOpen className="h-3.5 w-3.5" />
          Facts
        </button>
        <button className="flex flex-1 items-center justify-center gap-1.5 px-3 py-2.5 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors">
          <Clock className="h-3.5 w-3.5" />
          History
        </button>
      </div>

      {/* Placeholder content */}
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
        <p className="text-sm font-medium">Coming soon</p>
        <p className="text-xs opacity-70">
          Annotations, facts, and timeline will be available in a future update.
        </p>
      </div>
    </div>
  );
}
