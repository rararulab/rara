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

import { useCallback, useRef, useState } from "react";
import type {
  Actor,
  DockAnnotation,
  DockBlock,
  DockFact,
  DockHistoryEntry,
  DockMutation,
  DockSessionMeta,
  DockTurnResponse,
} from "@/api/dock";
import {
  dockBootstrap,
  dockCreateSession,
  dockGetSession,
  dockMutateSession,
  dockTurnStream,
  dockUpdateWorkspace,
} from "@/api/dock";

// ---------------------------------------------------------------------------
// Public store interface
// ---------------------------------------------------------------------------

export interface DockStore {
  // Session state
  sessions: DockSessionMeta[];
  activeSessionId: string | null;

  // Canvas state
  blocks: DockBlock[];
  annotations: DockAnnotation[];
  facts: DockFact[];
  activeAnnotation: string | null;

  // History
  history: DockHistoryEntry[];
  selectedAnchor: string | null;

  // UI state
  isRunning: boolean;
  error: string | null;
  rightPanelOpen: boolean;
  activeTab: "annotations" | "facts" | "history";

  // UI actions
  setActiveTab(tab: "annotations" | "facts" | "history"): void;
  setActiveAnnotation(id: string | null): void;

  // Actions
  bootstrap(): Promise<void>;
  selectSession(id: string): Promise<void>;
  newSession(): Promise<void>;
  renameSession(id: string): Promise<void>;
  sendMessage(text: string, isCommand?: boolean): Promise<void>;
  selectHistoryAnchor(name: string): Promise<void>;

  // Block operations
  addBlock(type: string, html: string, id?: string): string;
  updateBlock(id: string, html: string, author?: Actor): void;
  removeBlock(id: string): void;
  dismissDiff(id: string): void;

  // Annotation operations
  addAnnotation(annotation: Partial<DockAnnotation>): DockAnnotation;
  updateAnnotation(id: string, content: string): void;
  removeAnnotation(id: string): void;

  // Fact operations
  addFact(content: string, source?: Actor): void;
  updateFact(id: string, content: string): void;
  removeFact(id: string): void;

  // Mutation handling
  applyDockPayload(payload: DockTurnResponse): void;
  applyDockMutation(mutation: DockMutation): void;

  // Utility
  formatTime(value: string | number): string;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function generateId(): string {
  return crypto.randomUUID();
}

function formatTime(value: string | number): string {
  const ts =
    typeof value === "string" ? new Date(value).getTime() : value;
  const now = Date.now();
  const diffSec = Math.floor((now - ts) / 1000);

  if (diffSec < 60) return "just now";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;

  const d = new Date(ts);
  const months = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ];
  return `${months[d.getMonth()]} ${d.getDate()}`;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useDockStore(): DockStore {
  const [sessions, setSessions] = useState<DockSessionMeta[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);

  const [blocks, setBlocks] = useState<DockBlock[]>([]);
  const [annotations, setAnnotations] = useState<DockAnnotation[]>([]);
  const [facts, setFacts] = useState<DockFact[]>([]);
  const [activeAnnotation, setActiveAnnotation] = useState<string | null>(null);

  const [history, setHistory] = useState<DockHistoryEntry[]>([]);
  const [selectedAnchor, setSelectedAnchor] = useState<string | null>(null);

  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rightPanelOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<"annotations" | "facts" | "history">("annotations");

  // Refs to provide stable access to latest state inside callbacks
  const blocksRef = useRef(blocks);
  blocksRef.current = blocks;
  const factsRef = useRef(facts);
  factsRef.current = facts;
  const annotationsRef = useRef(annotations);
  annotationsRef.current = annotations;
  const activeSessionIdRef = useRef(activeSessionId);
  activeSessionIdRef.current = activeSessionId;
  const selectedAnchorRef = useRef(selectedAnchor);
  selectedAnchorRef.current = selectedAnchor;

  // -----------------------------------------------------------------------
  // Mutation handling
  // -----------------------------------------------------------------------

  const applyDockMutation = useCallback((mutation: DockMutation) => {
    const { op } = mutation;

    switch (op) {
      case "block.add":
        if (mutation.block) {
          setBlocks((prev) => [...prev, mutation.block!]);
        }
        break;
      case "block.update":
        if (mutation.block) {
          const updated = mutation.block;
          setBlocks((prev) =>
            prev.map((b) => (b.id === updated.id ? { ...b, ...updated } : b)),
          );
        }
        break;
      case "block.remove":
        if (mutation.id) {
          setBlocks((prev) => prev.filter((b) => b.id !== mutation.id));
        }
        break;
      case "fact.add":
        if (mutation.fact) {
          setFacts((prev) => [...prev, mutation.fact!]);
        }
        break;
      case "fact.update":
        if (mutation.fact) {
          const updated = mutation.fact;
          setFacts((prev) =>
            prev.map((f) => (f.id === updated.id ? { ...f, ...updated } : f)),
          );
        }
        break;
      case "fact.remove":
        if (mutation.id) {
          setFacts((prev) => prev.filter((f) => f.id !== mutation.id));
        }
        break;
      case "annotation.add":
        if (mutation.annotation) {
          setAnnotations((prev) => [...prev, mutation.annotation!]);
        }
        break;
      case "annotation.update":
        if (mutation.annotation) {
          const updated = mutation.annotation;
          setAnnotations((prev) =>
            prev.map((a) => (a.id === updated.id ? { ...a, ...updated } : a)),
          );
        }
        break;
      case "annotation.remove":
        if (mutation.id) {
          setAnnotations((prev) => prev.filter((a) => a.id !== mutation.id));
        }
        break;
      default:
        break;
    }
  }, []);

  const applyDockPayload = useCallback(
    (payload: DockTurnResponse) => {
      // Apply incremental mutations first
      for (const m of payload.mutations) {
        applyDockMutation(m);
      }

      // Then set authoritative state from response
      if (Array.isArray(payload.blocks)) {
        setBlocks(payload.blocks);
      }
      if (Array.isArray(payload.facts)) {
        setFacts(payload.facts);
      }
      if (Array.isArray(payload.annotations)) {
        setAnnotations(payload.annotations);
      }
      if (Array.isArray(payload.history)) {
        setHistory(payload.history);
      }
      if (payload.selected_anchor !== undefined) {
        setSelectedAnchor(payload.selected_anchor ?? null);
      }
      if (payload.session) {
        setSessions((prev) =>
          prev.map((s) =>
            s.id === payload.session!.id ? payload.session! : s,
          ),
        );
      }
    },
    [applyDockMutation],
  );

  // -----------------------------------------------------------------------
  // Session actions
  // -----------------------------------------------------------------------

  const loadSession = useCallback(
    async (id: string, anchor?: string) => {
      try {
        const resp = await dockGetSession(id, anchor);
        setBlocks(resp.blocks);
        setAnnotations(resp.annotations);
        setFacts(resp.facts);
        setHistory(resp.history);
        setSelectedAnchor(resp.selected_anchor ?? null);
        setActiveAnnotation(null);
        setError(null);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to load session");
      }
    },
    [],
  );

  const bootstrap = useCallback(async () => {
    try {
      const resp = await dockBootstrap();
      setSessions(resp.sessions);

      if (resp.active_session_id) {
        setActiveSessionId(resp.active_session_id);
        await loadSession(resp.active_session_id);
      } else if (resp.sessions.length > 0) {
        const first = resp.sessions[0];
        setActiveSessionId(first.id);
        await dockUpdateWorkspace(first.id);
        await loadSession(first.id);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to bootstrap dock");
    }
  }, [loadSession]);

  const selectSession = useCallback(
    async (id: string) => {
      setActiveSessionId(id);
      await dockUpdateWorkspace(id);
      await loadSession(id);
    },
    [loadSession],
  );

  const newSession = useCallback(async () => {
    try {
      const meta = await dockCreateSession();
      setSessions((prev) => [meta, ...prev]);
      setActiveSessionId(meta.id);
      await dockUpdateWorkspace(meta.id);
      setBlocks([]);
      setAnnotations([]);
      setFacts([]);
      setHistory([]);
      setSelectedAnchor(null);
      setActiveAnnotation(null);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create session");
    }
  }, []);

  const renameSession = useCallback(async (_id: string) => {
    // Rename is a UI-only action for now; will be wired to API later
  }, []);

  const sendMessage = useCallback(
    async (text: string, isCommand = false) => {
      const sid = activeSessionIdRef.current;
      if (!sid) return;

      setIsRunning(true);
      setError(null);

      try {
        await dockTurnStream(
          {
            session_id: sid,
            content: text,
            is_command: isCommand,
            blocks: blocksRef.current,
            facts: factsRef.current,
            annotations: annotationsRef.current,
            selected_anchor: selectedAnchorRef.current ?? undefined,
          },
          (event) => {
            switch (event.type) {
              case "dock_turn_complete":
                applyDockPayload(event.data);
                break;
              case "error":
                setError(event.error);
                break;
              case "done":
                setIsRunning(false);
                break;
            }
          },
        );
      } catch (err) {
        setError(err instanceof Error ? err.message : "Turn failed");
      } finally {
        setIsRunning(false);
      }
    },
    [applyDockPayload],
  );

  const selectHistoryAnchor = useCallback(
    async (name: string) => {
      const sid = activeSessionIdRef.current;
      if (!sid) return;
      setSelectedAnchor(name);
      await loadSession(sid, name);
    },
    [loadSession],
  );

  // -----------------------------------------------------------------------
  // Block operations
  // -----------------------------------------------------------------------

  const addBlock = useCallback(
    (type: string, html: string, id?: string): string => {
      const blockId = id ?? generateId();
      const block: DockBlock = { id: blockId, block_type: type, html };
      setBlocks((prev) => [...prev, block]);
      return blockId;
    },
    [],
  );

  const updateBlock = useCallback(
    (id: string, html: string, author?: Actor) => {
      setBlocks((prev) =>
        prev.map((b) => {
          if (b.id !== id) return b;
          const diff: DockBlock["diff"] = author
            ? { original: b.html, modified: html, author }
            : b.diff;
          return { ...b, html, diff };
        }),
      );
    },
    [],
  );

  const removeBlock = useCallback((id: string) => {
    setBlocks((prev) => prev.filter((b) => b.id !== id));
  }, []);

  const dismissDiff = useCallback((id: string) => {
    setBlocks((prev) =>
      prev.map((b) => (b.id === id ? { ...b, diff: undefined } : b)),
    );
  }, []);

  // -----------------------------------------------------------------------
  // Annotation operations
  // -----------------------------------------------------------------------

  const addAnnotation = useCallback(
    (partial: Partial<DockAnnotation>): DockAnnotation => {
      const annotation: DockAnnotation = {
        id: partial.id ?? generateId(),
        block_id: partial.block_id ?? "",
        content: partial.content ?? "",
        author: partial.author ?? "human",
        anchor_y: partial.anchor_y ?? 0,
        timestamp: partial.timestamp ?? Date.now(),
        selection: partial.selection ?? null,
      };
      setAnnotations((prev) => [...prev, annotation]);
      const sid = activeSessionIdRef.current;
      if (sid) {
        dockMutateSession(sid, [
          { op: "annotation.add", actor: "human", annotation },
        ]).catch(() => {});
      }
      return annotation;
    },
    [],
  );

  const updateAnnotation = useCallback((id: string, content: string) => {
    setAnnotations((prev) =>
      prev.map((a) => (a.id === id ? { ...a, content } : a)),
    );
    const sid = activeSessionIdRef.current;
    if (sid) {
      const ann = annotationsRef.current.find((a) => a.id === id);
      if (ann) {
        dockMutateSession(sid, [
          { op: "annotation.update", actor: "human", annotation: { ...ann, content } },
        ]).catch(() => {});
      }
    }
  }, []);

  const removeAnnotation = useCallback((id: string) => {
    setAnnotations((prev) => prev.filter((a) => a.id !== id));
    const sid = activeSessionIdRef.current;
    if (sid) {
      dockMutateSession(sid, [
        { op: "annotation.remove", actor: "human", id },
      ]).catch(() => {});
    }
  }, []);

  // -----------------------------------------------------------------------
  // Fact operations
  // -----------------------------------------------------------------------

  const addFact = useCallback(
    (content: string, source: Actor = "human") => {
      const fact: DockFact = { id: generateId(), content, source };
      setFacts((prev) => [...prev, fact]);
      const sid = activeSessionIdRef.current;
      if (sid) {
        dockMutateSession(sid, [
          { op: "fact.add", actor: "human", fact },
        ]).catch(() => {});
      }
    },
    [],
  );

  const updateFact = useCallback((id: string, content: string) => {
    setFacts((prev) =>
      prev.map((f) => (f.id === id ? { ...f, content } : f)),
    );
    const sid = activeSessionIdRef.current;
    if (sid) {
      const f = factsRef.current.find((f) => f.id === id);
      if (f) {
        dockMutateSession(sid, [
          { op: "fact.update", actor: "human", fact: { ...f, content } },
        ]).catch(() => {});
      }
    }
  }, []);

  const removeFact = useCallback((id: string) => {
    setFacts((prev) => prev.filter((f) => f.id !== id));
    const sid = activeSessionIdRef.current;
    if (sid) {
      dockMutateSession(sid, [
        { op: "fact.remove", actor: "human", id },
      ]).catch(() => {});
    }
  }, []);

  // -----------------------------------------------------------------------
  // Return store
  // -----------------------------------------------------------------------

  return {
    sessions,
    activeSessionId,
    blocks,
    annotations,
    facts,
    activeAnnotation,
    history,
    selectedAnchor,
    isRunning,
    error,
    rightPanelOpen,
    activeTab,

    setActiveTab,
    setActiveAnnotation,

    bootstrap,
    selectSession,
    newSession,
    renameSession,
    sendMessage,
    selectHistoryAnchor,

    addBlock,
    updateBlock,
    removeBlock,
    dismissDiff,

    addAnnotation,
    updateAnnotation,
    removeAnnotation,

    addFact,
    updateFact,
    removeFact,

    applyDockPayload,
    applyDockMutation,

    formatTime,
  };
}
