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

import { useState } from 'react';
import { ChevronDown, Loader2, PanelRightClose, PanelRightOpen, Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import type { DockStore } from '@/hooks/use-dock-store';

interface DockHeaderProps {
  store: DockStore;
  rightPanelOpen: boolean;
  onToggleRightPanel: () => void;
}

export default function DockHeader({ store, rightPanelOpen, onToggleRightPanel }: DockHeaderProps) {
  const [dropdownOpen, setDropdownOpen] = useState(false);

  const activeSession = store.sessions.find((s) => s.id === store.activeSessionId);

  return (
    <div className="flex shrink-0 items-center justify-between border-b border-border/40 bg-background/50 px-4 py-2 backdrop-blur-sm">
      {/* Left: session switcher */}
      <div className="relative flex items-center gap-2">
        <div className="relative">
          <button
            className="flex items-center gap-1.5 rounded-md px-2 py-1 text-sm font-medium hover:bg-accent/50 transition-colors"
            onClick={() => setDropdownOpen(!dropdownOpen)}
          >
            <span className="max-w-[200px] truncate">{activeSession?.title || 'No session'}</span>
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          </button>

          {dropdownOpen && (
            <>
              <div className="fixed inset-0 z-40" onClick={() => setDropdownOpen(false)} />
              <div className="absolute left-0 top-full z-50 mt-1 min-w-[220px] rounded-lg border border-border/60 bg-card p-1 shadow-lg">
                {store.sessions.map((session) => (
                  <button
                    key={session.id}
                    className={cn(
                      'flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors hover:bg-accent/50',
                      session.id === store.activeSessionId && 'bg-accent/30 font-medium',
                    )}
                    onClick={() => {
                      store.selectSession(session.id);
                      setDropdownOpen(false);
                    }}
                  >
                    <span className="flex-1 truncate">{session.title}</span>
                    <span className="text-[10px] text-muted-foreground">
                      {store.formatTime(session.updated_at)}
                    </span>
                  </button>
                ))}
                <div className="my-1 border-t border-border/40" />
                <button
                  className="flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm text-muted-foreground transition-colors hover:bg-accent/50 hover:text-foreground"
                  onClick={() => {
                    store.newSession();
                    setDropdownOpen(false);
                  }}
                >
                  <Plus className="h-3.5 w-3.5" />
                  New session
                </button>
              </div>
            </>
          )}
        </div>

        {store.isRunning && (
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            <span>Running</span>
          </div>
        )}
      </div>

      {/* Right: controls */}
      <div className="flex items-center gap-1">
        {store.error && (
          <span className="mr-2 max-w-[200px] truncate text-xs text-destructive">
            {store.error}
          </span>
        )}
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          onClick={onToggleRightPanel}
          title={rightPanelOpen ? 'Close panel' : 'Open panel'}
        >
          {rightPanelOpen ? (
            <PanelRightClose className="h-4 w-4" />
          ) : (
            <PanelRightOpen className="h-4 w-4" />
          )}
        </Button>
      </div>
    </div>
  );
}
