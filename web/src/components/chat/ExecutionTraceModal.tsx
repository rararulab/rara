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
import type { ExecutionTrace, ToolTraceEntry } from "@/api/kernel-types";

/**
 * Execution-trace viewer — opened from the "📊 详情" button on a
 * completed assistant turn. Mirrors the Telegram renderer in
 * `crates/channels/src/telegram/adapter.rs::render_trace_detail` so the
 * two surfaces stay informationally consistent.
 *
 * Unlike the TG adapter (which truncates at 4000 chars to stay under
 * the 4096-char Telegram message limit) the web modal shows the full
 * trace — only individual long sections are collapsed behind an
 * expander so the modal stays scannable.
 */

/**
 * Inline collapse threshold (chars) — thinking preview longer than
 * this starts collapsed. Matches {@link CascadeModal}'s threshold so
 * the two modals feel consistent.
 */
const COLLAPSE_THRESHOLD = 600;

interface Props {
  open:    boolean;
  trace:   ExecutionTrace | null;
  loading: boolean;
  error:   string | null;
  onClose: () => void;
}

/** Modal wrapper + status switching (loading / error / empty / trace). */
export function ExecutionTraceModal({ open, trace, loading, error, onClose }: Props) {
  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="flex max-h-[85vh] w-[95vw] max-w-3xl flex-col gap-3 overflow-hidden">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <span aria-hidden>📊</span>
            <span>Execution Trace</span>
          </DialogTitle>
          {trace && (
            <DialogDescription>
              {formatSummary(trace)}
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
          {!loading && !error && trace && <TraceBody trace={trace} />}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function formatSummary(trace: ExecutionTrace): string {
  const dur = `${trace.duration_secs}s`;
  const tok = `↑${trace.input_tokens} ↓${trace.output_tokens}`;
  const thinking = trace.thinking_ms > 0
    ? ` · thought ${(trace.thinking_ms / 1000).toFixed(1)}s`
    : "";
  return `${dur} · ${tok}${thinking}`;
}

function TraceBody({ trace }: { trace: ExecutionTrace }) {
  return (
    <div className="flex flex-col gap-4 text-sm">
      {trace.turn_rationale && trace.turn_rationale.trim().length > 0 && (
        <Section emoji="💭" title="Rationale">
          <blockquote className="border-l-2 border-border/60 pl-3 text-foreground/85 whitespace-pre-wrap break-words">
            {trace.turn_rationale}
          </blockquote>
        </Section>
      )}

      {trace.thinking_preview.trim().length > 0 && (
        <Section
          emoji="🧠"
          title={`Thinking (${(trace.thinking_ms / 1000).toFixed(1)}s)`}
        >
          <Collapsible text={trace.thinking_preview} />
        </Section>
      )}

      {trace.plan_steps.length > 0 && (
        <Section emoji="📋" title="Plan">
          <ol className="ml-5 list-decimal space-y-1 text-foreground/85">
            {trace.plan_steps.map((step, i) => (
              <li key={i} className="whitespace-pre-wrap break-words">{step}</li>
            ))}
          </ol>
        </Section>
      )}

      {trace.tools.length > 0 && (
        <Section emoji="🔧" title="Tools">
          <ul className="flex flex-col gap-1.5 border-l-2 border-border/60 pl-3">
            {trace.tools.map((t, i) => <ToolRow key={i} tool={t} />)}
          </ul>
        </Section>
      )}

      <Section emoji="📊" title="Usage">
        <div className="text-foreground/85">
          {trace.iterations} iterations
          {" · "}↑{trace.input_tokens}
          {" "}↓{trace.output_tokens} tokens
          {trace.model && <> · <code className="rounded bg-muted/60 px-1 py-0.5 font-mono text-xs">{trace.model}</code></>}
        </div>
      </Section>

      <Section emoji="🆔" title="Message ID">
        <code className="block rounded bg-muted/60 px-2 py-1 font-mono text-xs break-all">
          {trace.rara_message_id}
        </code>
      </Section>
    </div>
  );
}

function Section({
  emoji,
  title,
  children,
}: {
  emoji:    string;
  title:    string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-1.5">
      <header className="flex items-center gap-2 text-sm font-semibold">
        <span aria-hidden>{emoji}</span>
        <span>{title}</span>
      </header>
      <div className="text-sm">{children}</div>
    </section>
  );
}

function ToolRow({ tool }: { tool: ToolTraceEntry }) {
  const icon = tool.success ? "✓" : "✗";
  const iconCls = tool.success ? "text-emerald-600" : "text-destructive";
  const dur =
    tool.duration_ms !== null && tool.duration_ms !== undefined
      ? formatDuration(tool.duration_ms)
      : null;
  return (
    <li className="flex flex-col gap-0.5">
      <div className="flex flex-wrap items-center gap-x-2 text-foreground/90">
        <span className={`font-mono text-sm ${iconCls}`} aria-hidden>{icon}</span>
        <code className="font-mono text-xs font-semibold">{tool.name}</code>
        {dur && (
          <span className="rounded bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
            {dur}
          </span>
        )}
        {tool.summary && (
          <span className="text-foreground/80">— {tool.summary}</span>
        )}
      </div>
      {tool.error && (
        <div className="ml-5 rounded border border-destructive/30 bg-destructive/10 px-2 py-1 text-xs text-destructive whitespace-pre-wrap break-words">
          ⚠ {tool.error}
        </div>
      )}
    </li>
  );
}

function Collapsible({ text }: { text: string }) {
  const [expanded, setExpanded] = useState(false);
  const collapsible = text.length > COLLAPSE_THRESHOLD;
  const body = !collapsible || expanded
    ? text
    : text.slice(0, COLLAPSE_THRESHOLD) + "…";
  return (
    <div>
      <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground/85">
        {body}
      </pre>
      {collapsible && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="mt-1 text-xs text-primary hover:underline"
        >
          {expanded ? "收起" : `展开（共 ${text.length} 字符）`}
        </button>
      )}
    </div>
  );
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}m${s}s`;
}
