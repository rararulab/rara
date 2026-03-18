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

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type {
  CascadeEntry,
  CascadeTrace,
} from "@/lib/cascade-types";

/** Tool call currently in progress, mirroring Chat.tsx's ActiveToolCall shape. */
export interface CascadeActiveToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

/** Completed tool call result, mirroring Chat.tsx's CompletedTool shape. */
export interface CascadeCompletedTool {
  id: string;
  name: string;
  success: boolean;
  result_preview: string;
  error: string | null;
}

/** Streaming state forwarded from the parent chat component. */
export interface CascadeStreamState {
  reasoning: string;
  activeTools: CascadeActiveToolCall[];
  completedTools: CascadeCompletedTool[];
}

interface UseCascadeOptions {
  sessionKey: string;
  messageSeq: number;
  isStreaming: boolean;
  streamState?: CascadeStreamState;
}

/** Build a live CascadeTrace from the current stream state props. */
function buildLiveTrace(streamState: CascadeStreamState): CascadeTrace {
  const now = new Date().toISOString();
  const entries: CascadeEntry[] = [];

  // Reasoning becomes a thought entry
  if (streamState.reasoning) {
    entries.push({
      kind: "thought",
      id: "thk • LIVE-reasoning",
      content: streamState.reasoning,
      timestamp: now,
    });
  }

  // Completed tools become action + observation pairs
  for (const tool of streamState.completedTools) {
    entries.push({
      kind: "action",
      id: `act • ${tool.id}`,
      content: `${tool.name}()`,
      timestamp: now,
    });
    entries.push({
      kind: "observation",
      id: `obs • ${tool.id}`,
      content: tool.error ?? tool.result_preview,
      timestamp: now,
      metadata: { success: tool.success },
    });
  }

  // Active (in-flight) tools shown as pending actions
  for (const tool of streamState.activeTools) {
    entries.push({
      kind: "action",
      id: `act • ${tool.id}`,
      content: `${tool.name}(…) [running]`,
      timestamp: now,
      metadata: { arguments: tool.arguments, status: "running" },
    });
  }

  return {
    message_id: "streaming",
    ticks: [{ index: 0, entries }],
    summary: {
      tick_count: 1,
      tool_call_count: streamState.completedTools.length + streamState.activeTools.length,
      total_entries: entries.length,
    },
  };
}

/**
 * Hook that provides cascade trace data for either historical (REST) or
 * live-streaming mode. When `isStreaming` is true and `streamState` is
 * provided, a live trace is synthesized from the stream props. Otherwise
 * the trace is fetched via the REST API.
 */
export function useCascade({
  sessionKey,
  messageSeq,
  isStreaming,
  streamState,
}: UseCascadeOptions) {
  // REST fetch for historical messages (disabled while streaming)
  const {
    data: restTrace,
    isLoading,
    error,
  } = useQuery<CascadeTrace>({
    queryKey: ["cascade-trace", sessionKey, messageSeq],
    queryFn: () =>
      api.get<CascadeTrace>(
        `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/trace?seq=${messageSeq}`,
      ),
    enabled: !isStreaming,
  });

  // Build live trace reactively from streamState props
  const liveTrace = useMemo(() => {
    if (!isStreaming || !streamState) return null;
    return buildLiveTrace(streamState);
  }, [isStreaming, streamState]);

  return {
    trace: isStreaming ? liveTrace : restTrace ?? null,
    isLoading: !isStreaming && isLoading,
    error: !isStreaming ? error : null,
  };
}
