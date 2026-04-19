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

import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowUp, Loader2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import type { DockStore } from '@/hooks/use-dock-store';

interface DockConsoleProps {
  store: DockStore;
}

export default function DockConsole({ store }: DockConsoleProps) {
  const [input, setInput] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Auto-resize textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [input]);

  const handleSubmit = useCallback(() => {
    const text = input.trim();
    if (!text || store.isRunning) return;

    const isCommand = text.startsWith(',');
    store.sendMessage(text, isCommand);
    setInput('');
  }, [input, store]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  return (
    <div className="shrink-0 border-t border-border/40 bg-background/50 backdrop-blur-sm">
      {store.isRunning && (
        <div className="flex items-center gap-2 px-4 py-1.5 text-xs text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin" />
          Agent is working...
        </div>
      )}
      <div className="flex items-end gap-2 p-3">
        <textarea
          ref={textareaRef}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            store.activeSessionId ? 'Message or ,command...' : 'Create a session to start'
          }
          disabled={store.isRunning || !store.activeSessionId}
          rows={1}
          className="flex-1 resize-none rounded-lg border border-border/60 bg-card/60 px-3 py-2 text-sm placeholder:text-muted-foreground/60 focus:border-ring focus:outline-none disabled:opacity-50"
        />
        <Button
          size="icon"
          className="h-9 w-9 shrink-0"
          disabled={!input.trim() || store.isRunning || !store.activeSessionId}
          onClick={handleSubmit}
        >
          <ArrowUp className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
