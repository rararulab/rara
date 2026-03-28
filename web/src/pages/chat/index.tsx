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

import { useCallback, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { MessageTracePanel } from "@/components/MessageTracePanel";
import type { CascadeStreamState } from "@/hooks/use-cascade";
import type { ChatSession } from "@/api/types";
import { cn } from "@/lib/utils";
import type { PendingDraft } from "./types";
import { createSession, clearMessages, deleteSession, fetchSessions } from "./api";
import { generateKey } from "./utils";
import { SessionList } from "./SessionSidebar";
import { NewChatDialog } from "./ModelPicker";
import { ChatThread } from "./ChatThread";
import { EmptyState } from "./EmptyState";

// ---------------------------------------------------------------------------
// Chat (main page component)
// ---------------------------------------------------------------------------

export default function Chat({
  onOpenOperations,
}: {
  onOpenOperations?: () => void;
} = {}) {
  const queryClient = useQueryClient();
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [panelCollapsed, setPanelCollapsed] = useState(false);
  const [newChatDialogOpen, setNewChatDialogOpen] = useState(false);
  const [pendingDraft, setPendingDraft] = useState<PendingDraft | null>(null);
  const [cascadeSeq, setCascadeSeq] = useState<number | null>(null);
  const [threadStreaming, setThreadStreaming] = useState(false);
  const [threadStreamState, setThreadStreamState] = useState<CascadeStreamState>({
    reasoning: "",
    activeTools: [],
    completedTools: [],
  });

  const handleStreamStateChange = useCallback(
    (isStreaming: boolean, state: CascadeStreamState) => {
      setThreadStreaming(isStreaming);
      setThreadStreamState(state);
    },
    [],
  );

  // Clear cascade panel when switching sessions
  const switchSession = useCallback((key: string | null) => {
    setActiveKey(key);
    setCascadeSeq(null);
  }, []);

  const sessionsQuery = useQuery({
    queryKey: ["chat-sessions"],
    queryFn: fetchSessions,
  });

  // Hide internal agent sessions from the UI
  const sessions = (sessionsQuery.data ?? []).filter(
    (s) => !s.key.startsWith("agent:"),
  );

  const activeSession = activeKey
    ? sessions.find((s) => s.key === activeKey) ?? null
    : null;

  const createMutation = useMutation({
    mutationFn: createSession,
    onSuccess: (session) => {
      queryClient.setQueryData<ChatSession[]>(["chat-sessions"], (old) => {
        const next = old ?? [];
        if (next.some((s) => s.key === session.key)) return next;
        return [session, ...next];
      });
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      switchSession(session.key);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: deleteSession,
    onSuccess: (_data, deletedKey) => {
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      queryClient.removeQueries({
        queryKey: ["chat-messages", deletedKey],
      });
      if (activeKey === deletedKey) {
        switchSession(null);
      }
    },
  });

  const clearMutation = useMutation({
    mutationFn: clearMessages,
    onSuccess: (_data, clearedKey) => {
      queryClient.invalidateQueries({
        queryKey: ["chat-messages", clearedKey],
      });
      queryClient.setQueryData<ChatSession[]>(["chat-sessions"], (old) =>
        old?.map((s) =>
          s.key === clearedKey
            ? { ...s, message_count: 0, preview: null }
            : s,
        ),
      );
    },
  });

  const handleCreateConfirm = useCallback(
    (title: string, model: string) => {
      const key = generateKey();
      createMutation.mutate({ key, title, model });
    },
    [createMutation],
  );

  const handleStartFromEmpty = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed) return;
      const key = generateKey();
      setPendingDraft({ text: trimmed });
      try {
        await createMutation.mutateAsync({
          key,
          title: trimmed.slice(0, 80),
        });
      } catch {
        setPendingDraft(null);
      }
    },
    [createMutation],
  );

  const handleDelete = useCallback(
    (key: string) => {
      deleteMutation.mutate(key);
    },
    [deleteMutation],
  );

  const handleClearMessages = useCallback(() => {
    if (!activeKey) return;
    clearMutation.mutate(activeKey);
  }, [activeKey, clearMutation]);

  return (
    <div className="relative flex h-full overflow-hidden">
      {/* New chat dialog */}
      <NewChatDialog
        open={newChatDialogOpen}
        onOpenChange={setNewChatDialogOpen}
        onConfirm={handleCreateConfirm}
      />

      {/* Left panel: session list */}
      <SessionList
        sessions={sessions}
        activeKey={activeKey}
        onSelect={switchSession}
        onDelete={handleDelete}
        isLoading={sessionsQuery.isLoading}
        collapsed={panelCollapsed}
        onToggleCollapse={() => setPanelCollapsed((p) => !p)}
        onOpenOperations={() => onOpenOperations?.()}
      />

      <div
        className={cn(
          "flex h-full min-w-0 flex-1 p-2 md:p-3 transition-[padding] duration-200",
          panelCollapsed ? "" : "md:pl-[17.75rem]",
        )}
      >
        <div className="flex min-w-0 flex-1 overflow-hidden rounded-2xl bg-transparent">
          {/* Right panel: chat thread or empty state */}
          {activeSession ? (
            <div className="flex min-w-0 flex-1 overflow-hidden">
              <div className="min-w-0 flex-1">
                <ChatThread
                  key={activeKey}
                  session={activeSession}
                  onClearMessages={handleClearMessages}
                  panelCollapsed={panelCollapsed}
                  onTogglePanel={() => setPanelCollapsed((p) => !p)}
                  initialDraft={pendingDraft}
                  onInitialDraftConsumed={() => setPendingDraft(null)}
                  onMessageClick={(seq) => setCascadeSeq(seq)}
                  onStreamStateChange={handleStreamStateChange}
                />
              </div>
              {cascadeSeq !== null && activeKey && (
                <MessageTracePanel
                  sessionKey={activeKey}
                  messageSeq={cascadeSeq}
                  isStreaming={threadStreaming}
                  streamState={threadStreamState}
                  onClose={() => setCascadeSeq(null)}
                />
              )}
            </div>
          ) : (
            <EmptyState
              onSendFirstMessage={handleStartFromEmpty}
              panelCollapsed={panelCollapsed}
              onTogglePanel={() => setPanelCollapsed((p) => !p)}
            />
          )}
        </div>
      </div>
    </div>
  );
}
