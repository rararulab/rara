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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type {
  CascadeEntry,
  CascadeEntryKind,
  CascadeTrace,
} from "@/api/kernel-types";

/**
 * Cascade execution-trace viewer — opened from the "📊 详情" button on a
 * completed assistant turn. Mirrors the Telegram renderer in
 * `crates/channels/src/telegram/adapter.rs::render_cascade_html` so the two
 * surfaces stay visually consistent.
 *
 * The trace is fetched lazily by the parent (PiChat) once the user opens
 * the modal — the kernel does not stream cascade data, it only assembles it
 * after a turn finishes (see `service.get_cascade_trace`).
 */

const ENTRY_LABELS: Record<CascadeEntryKind, { emoji: string; label: string }> = {
  user_input:  { emoji: "💬", label: "User Input" },
  thought:     { emoji: "🧠", label: "Thought" },
  action:      { emoji: "⚡", label: "Action" },
  observation: { emoji: "👁", label: "Observation" },
};

/**
 * Inline collapse threshold (chars) — entries longer than this start
 * collapsed and reveal the full body when "展开" is clicked. The TG
 * adapter caps at 4000 chars *total*; web has no such hard limit, so we
 * just hide individual long bodies behind a toggle to keep the modal
 * scannable while still allowing inspection of any single entry.
 */
const COLLAPSE_THRESHOLD = 600;

interface Props {
  open:    boolean;
  trace:   CascadeTrace | null;
  loading: boolean;
  error:   string | null;
  onClose: () => void;
}

/** Modal wrapper + status switching (loading / error / empty / trace). */
export function CascadeModal({ open, trace, loading, error, onClose }: Props) {
  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="flex max-h-[85vh] w-[95vw] max-w-3xl flex-col gap-3 overflow-hidden">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <span aria-hidden>🔍</span>
            <span>Cascade Trace</span>
          </DialogTitle>
          {trace && (
            <DialogDescription>
              {trace.summary.tick_count} ticks · {trace.summary.tool_call_count} tool calls · {trace.summary.total_entries} entries
            </DialogDescription>
          )}
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto pr-1">
          {loading && (
            <div className="flex items-center justify-center py-10 text-sm text-muted-foreground">
              <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-muted-foreground/30 border-t-muted-foreground" />
              加载中…
            </div>
          )}
          {!loading && error && (
            <div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
              加载失败：{error}
            </div>
          )}
          {!loading && !error && trace && trace.ticks.length === 0 && (
            <div className="py-10 text-center text-sm text-muted-foreground">
              该轮未记录任何 cascade 条目。
            </div>
          )}
          {!loading && !error && trace && trace.ticks.length > 0 && (
            <TraceBody trace={trace} />
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function TraceBody({ trace }: { trace: CascadeTrace }) {
  return (
    <div className="flex flex-col gap-4">
      {trace.ticks.map((tick) => (
        <section key={tick.index} className="flex flex-col gap-2">
          <header className="sticky top-0 z-[1] flex items-center gap-2 bg-background/95 py-1 text-sm font-semibold backdrop-blur">
            <span aria-hidden>▶</span>
            <span>TICK {tick.index + 1}</span>
            <span className="text-xs font-normal text-muted-foreground">
              · {tick.entries.length} entries
            </span>
          </header>
          <div className="flex flex-col gap-2">
            {tick.entries.map((entry) => (
              <EntryCard key={entry.id} entry={entry} />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

function EntryCard({ entry }: { entry: CascadeEntry }) {
  const [expanded, setExpanded] = useState(false);
  const meta = ENTRY_LABELS[entry.kind];
  const collapsible = entry.content.length > COLLAPSE_THRESHOLD;
  const body = !collapsible || expanded
    ? entry.content
    : entry.content.slice(0, COLLAPSE_THRESHOLD) + "…";

  return (
    <article className="rounded-md border border-border/60 bg-muted/20 p-3">
      <header className="mb-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
        <span className="text-base leading-none" aria-hidden>{meta.emoji}</span>
        <span className="font-semibold text-foreground/90">{meta.label}</span>
        <code className="rounded bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
          {entry.id}
        </code>
      </header>
      {body.length === 0 ? (
        <p className="text-xs italic text-muted-foreground">(empty)</p>
      ) : (
        <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground/85">
          {body}
        </pre>
      )}
      {collapsible && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="mt-2 text-xs text-primary hover:underline"
        >
          {expanded ? "收起" : `展开（共 ${entry.content.length} 字符）`}
        </button>
      )}
    </article>
  );
}
