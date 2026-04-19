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

import { Clock } from 'lucide-react';

import type { DockStore } from '@/hooks/use-dock-store';
import { cn } from '@/lib/utils';

interface DockTimelineProps {
  store: DockStore;
}

export default function DockTimeline({ store }: DockTimelineProps) {
  const { history, selectedAnchor } = store;

  if (history.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
        <Clock className="h-8 w-8 opacity-30" />
        <p className="text-xs">Tape anchors will appear here</p>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {history.map((entry) => {
        const isSelected = selectedAnchor === entry.anchor_name;

        return (
          <div
            key={entry.id}
            className={cn(
              'cursor-pointer border-b border-border/30 px-3 py-2.5 transition-colors',
              isSelected ? 'bg-accent/40' : 'hover:bg-accent/20',
            )}
            onClick={() => store.selectHistoryAnchor(entry.anchor_name)}
          >
            <p className="text-xs font-medium leading-snug text-foreground line-clamp-2">
              {entry.label}
            </p>
            {entry.preview && (
              <p className="mt-0.5 text-[11px] leading-snug text-muted-foreground line-clamp-2">
                {entry.preview}
              </p>
            )}
            <div className="mt-1.5 flex items-center gap-2 text-[10px] text-muted-foreground/70">
              <span>{store.formatTime(entry.timestamp)}</span>
              <span>&middot;</span>
              <span className="truncate font-mono">{entry.anchor_name}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
