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

import { useCallback, useEffect, useRef, useState } from "react";
import { Bot, Send } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useServerStatus } from "@/hooks/use-server-status";
import { ConversationPanelToggleButton } from "./SessionSidebar";

// ---------------------------------------------------------------------------
// EmptyState (when no session is selected)
// ---------------------------------------------------------------------------

export function EmptyState({
  onSendFirstMessage,
  panelCollapsed,
  onTogglePanel,
}: {
  onSendFirstMessage: (text: string) => void;
  panelCollapsed: boolean;
  onTogglePanel: () => void;
}) {
  const { isOnline } = useServerStatus();
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text || !isOnline) return;
    onSendFirstMessage(text);
    setInput("");
  }, [input, isOnline, onSendFirstMessage]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [input]);

  return (
    <div className="relative flex flex-1 flex-col">
      {panelCollapsed && (
        <div className="absolute left-4 top-4">
          <ConversationPanelToggleButton collapsed onToggle={onTogglePanel} />
        </div>
      )}

      <div className="flex flex-1 flex-col items-center justify-center gap-6">
        <div className="chat-empty-logo">
          <Bot className="h-8 w-8 text-white" />
        </div>
        <div className="text-center space-y-2">
          <h2 className="text-lg font-semibold text-foreground">Start a conversation</h2>
          <p className="text-sm text-muted-foreground max-w-sm">
            Ask anything — rara can help with coding, analysis, creative tasks, and more.
          </p>
        </div>
        <div className="flex flex-wrap justify-center gap-2">
          <span className="chat-empty-hint">
            <kbd className="rounded bg-muted px-1 font-mono text-[10px]">Enter</kbd> to send
          </span>
          <span className="chat-empty-hint">
            <kbd className="rounded bg-muted px-1 font-mono text-[10px]">Shift+Enter</kbd> new line
          </span>
        </div>
      </div>

      <div className="pointer-events-none absolute inset-x-4 bottom-4 z-10 md:inset-x-8 md:bottom-6">
        <div className="pointer-events-auto chat-composer flex items-end gap-2">
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={isOnline ? "Type a message... (Enter to send, Shift+Enter for newline)" : "Server offline -- sending disabled"}
            rows={1}
            disabled={!isOnline}
            autoFocus
            className="flex-1 resize-none appearance-none border-0 bg-transparent px-2 py-2.5 text-sm text-foreground shadow-none placeholder:text-muted-foreground focus:outline-none focus:ring-0 focus-visible:outline-none focus-visible:ring-0 disabled:cursor-not-allowed disabled:opacity-50"
          />
          <Button
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl bg-gradient-to-br from-primary to-pop-hover shadow-md shadow-primary/20 hover:shadow-lg hover:shadow-primary/30 transition-shadow"
            onClick={handleSend}
            disabled={!input.trim() || !isOnline}
            title={isOnline ? "Send message" : "Server offline"}
          >
            <Send className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  );
}
