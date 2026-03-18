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

import { useState } from "react";
import { ChevronDown, ChevronRight, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { JsonTree } from "@/components/JsonTree";
import { useCascade } from "@/hooks/use-cascade";
import type { CascadeStreamState } from "@/hooks/use-cascade";
import type {
  CascadeEntry,
  CascadeEntryKind,
  CascadeTick,
} from "@/lib/cascade-types";

interface MessageTracePanelProps {
  sessionKey: string;
  messageSeq: number;
  isStreaming: boolean;
  streamState?: CascadeStreamState;
  onClose: () => void;
}

/** Color mapping for each cascade entry kind. */
const KIND_COLORS: Record<CascadeEntryKind, string> = {
  user_input: "text-blue-400",
  thought: "text-yellow-400",
  action: "text-orange-400",
  observation: "text-emerald-400",
};

/** Renders a single cascade entry row with expandable data section. */
function EntryRow({ entry }: { entry: CascadeEntry }) {
  const [expanded, setExpanded] = useState(false);
  const idSuffix = entry.id.length > 8 ? entry.id.slice(-8) : entry.id;

  return (
    <div className="border-b border-zinc-800/50 last:border-b-0">
      <button
        className="flex w-full items-center gap-1.5 px-3 py-1 hover:bg-zinc-800/50 text-left"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded
          ? <ChevronDown className="h-3 w-3 shrink-0 text-zinc-500" />
          : <ChevronRight className="h-3 w-3 shrink-0 text-zinc-500" />}
        <span className={cn("font-semibold", KIND_COLORS[entry.kind])}>
          {entry.kind}
        </span>
        <span className="text-zinc-600 text-[10px]">{idSuffix}</span>
      </button>
      {expanded && (
        <div className="mx-3 mb-2 rounded bg-zinc-950 p-2 overflow-x-auto">
          <pre className="whitespace-pre-wrap break-words text-zinc-300">{entry.content}</pre>
          {entry.metadata && (
            <div className="mt-1 border-t border-zinc-800 pt-1">
              <JsonTree data={entry.metadata} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** Renders a single tick section with expandable entry list. */
function TickSection({ tick }: { tick: CascadeTick }) {
  const [expanded, setExpanded] = useState(true);

  return (
    <div className="border-b border-zinc-800">
      <button
        className="flex w-full items-center gap-1.5 px-3 py-1.5 hover:bg-zinc-800/30 text-left"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded
          ? <ChevronDown className="h-3 w-3 shrink-0 text-zinc-400" />
          : <ChevronRight className="h-3 w-3 shrink-0 text-zinc-400" />}
        <span className="font-bold text-zinc-200">TICK {tick.index}</span>
        <span className="text-zinc-600 text-[10px]">({tick.entries.length})</span>
      </button>
      {expanded && (
        <div className="pl-2">
          {tick.entries.map((entry) => (
            <EntryRow key={entry.id} entry={entry} />
          ))}
        </div>
      )}
    </div>
  );
}

/** Side panel that displays the cascade execution trace for an agent turn. */
export function MessageTracePanel({ sessionKey, messageSeq, isStreaming, streamState, onClose }: MessageTracePanelProps) {
  const { trace, isLoading, error } = useCascade({
    sessionKey,
    messageSeq,
    isStreaming,
    streamState,
  });

  return (
    <div className="flex h-full w-[480px] shrink-0 flex-col border-l border-zinc-800 bg-zinc-900 font-mono text-xs text-zinc-300">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-zinc-800 px-3 py-2">
        <span className="font-bold text-zinc-100">Cascade Viewer</span>
        <button
          className="rounded p-0.5 hover:bg-zinc-800 text-zinc-400 hover:text-zinc-200"
          onClick={onClose}
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Summary bar */}
      {trace && (
        <div className="border-b border-zinc-800 px-3 py-2 space-y-1">
          <div className="text-zinc-500 truncate">
            msg: <span className="text-zinc-300">{trace.message_id}</span>
          </div>
          <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-zinc-500">
            <span>ticks: <span className="text-zinc-300">{trace.summary.tick_count}</span></span>
            <span>tools: <span className="text-zinc-300">{trace.summary.tool_call_count}</span></span>
            <span>entries: <span className="text-zinc-300">{trace.summary.total_entries}</span></span>
          </div>
        </div>
      )}

      {/* Body */}
      <div className="flex-1 overflow-y-auto">
        {isLoading && (
          <div className="flex items-center justify-center py-8 text-zinc-500">
            Loading trace...
          </div>
        )}
        {error && (
          <div className="px-3 py-4 text-red-400">
            Failed to load trace: {error instanceof Error ? error.message : "Unknown error"}
          </div>
        )}
        {trace && trace.ticks.map((tick) => (
          <TickSection key={tick.index} tick={tick} />
        ))}
        {trace && trace.ticks.length === 0 && (
          <div className="flex items-center justify-center py-8 text-zinc-500">
            No ticks recorded.
          </div>
        )}
      </div>
    </div>
  );
}
