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

import { ApiError } from '@/api/client';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { useExecutionTrace } from '@/hooks/use-trace-fetch';

/**
 * Detect the seq-divergence 404 emitted by `get_execution_trace` when
 * the backend's two seq counters drift apart on a multi-tool turn (see
 * spec issue 2032 Decisions). The backend payload is the literal string
 * `"user message at seq <n> has no rara_turn_id metadata"`. Until the
 * backend is fixed, surface a friendly explanation rather than the raw
 * error body — every assistant turn with parallel tool results would
 * otherwise show a confusing internal error.
 */
function isSeqDivergence404(err: unknown): boolean {
  if (!(err instanceof ApiError)) return false;
  if (err.status !== 404) return false;
  return /rara_turn_id metadata/i.test(err.message);
}

export interface ExecutionTraceModalProps {
  /** Session key whose trace endpoint we hit. */
  sessionKey: string;
  /** Per-turn seq the trace is keyed on. `null` while the turn is still streaming. */
  seq: number | null;
  /** Whether the modal is open. The fetch hook is gated on this so the
   *  request only fires once the user actually opens the modal. */
  open: boolean;
  /** Close handler driven by the dialog's onOpenChange. */
  onOpenChange: (open: boolean) => void;
}

/**
 * Per-turn execution trace modal. Shows iteration count, model, token
 * usage, and per-tool summary for the assistant turn whose final
 * persisted seq matches `seq`. Layout is intentionally minimal — the
 * point is "data that was previously only reachable via curl is now
 * reachable from the UI" (goal.md signal 4), not visual polish.
 */
export function ExecutionTraceModal({
  sessionKey,
  seq,
  open,
  onOpenChange,
}: ExecutionTraceModalProps) {
  const { data, isLoading, isError, error } = useExecutionTrace(sessionKey, seq, open);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Turn execution trace</DialogTitle>
          <DialogDescription>
            seq {seq ?? '—'} · session <span className="font-mono text-xs">{sessionKey}</span>
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] overflow-y-auto text-sm">
          {isLoading && <div className="text-muted-foreground">Loading…</div>}
          {isError &&
            (isSeqDivergence404(error) ? (
              <div role="alert" className="text-muted-foreground">
                Trace data is not available for this turn yet.
              </div>
            ) : (
              <div role="alert" className="text-destructive">
                Failed to load trace: {error instanceof Error ? error.message : String(error)}
              </div>
            ))}
          {data && (
            <div className="space-y-3">
              <dl className="grid grid-cols-2 gap-x-4 gap-y-1">
                <dt className="text-muted-foreground">Model</dt>
                <dd className="font-mono">{data.model}</dd>
                <dt className="text-muted-foreground">Iterations</dt>
                <dd>{data.iterations}</dd>
                <dt className="text-muted-foreground">Duration</dt>
                <dd>{data.duration_secs.toFixed(2)}s</dd>
                <dt className="text-muted-foreground">Input tokens</dt>
                <dd>{data.input_tokens}</dd>
                <dt className="text-muted-foreground">Output tokens</dt>
                <dd>{data.output_tokens}</dd>
                <dt className="text-muted-foreground">Thinking</dt>
                <dd>{data.thinking_ms}ms</dd>
              </dl>
              {data.turn_rationale && (
                <section>
                  <h3 className="font-medium mb-1">Rationale</h3>
                  <p className="text-muted-foreground whitespace-pre-wrap">{data.turn_rationale}</p>
                </section>
              )}
              {data.thinking_preview && (
                <section>
                  <h3 className="font-medium mb-1">Thinking preview</h3>
                  <p className="text-muted-foreground whitespace-pre-wrap text-xs">
                    {data.thinking_preview}
                  </p>
                </section>
              )}
              {data.plan_steps.length > 0 && (
                <section>
                  <h3 className="font-medium mb-1">Plan</h3>
                  <ul className="list-disc pl-5 space-y-0.5">
                    {data.plan_steps.map((step, i) => (
                      <li key={i}>{step}</li>
                    ))}
                  </ul>
                </section>
              )}
              {data.tools.length > 0 && (
                <section>
                  <h3 className="font-medium mb-1">Tools</h3>
                  <ul className="space-y-1">
                    {data.tools.map((tool, i) => (
                      <li key={i} className="text-xs">
                        <span className="font-mono">{tool.name}</span>
                        {tool.duration_ms !== null && (
                          <span className="text-muted-foreground"> · {tool.duration_ms}ms</span>
                        )}
                        <span className={tool.success ? 'text-success' : 'text-destructive'}>
                          {' '}
                          · {tool.success ? 'ok' : 'failed'}
                        </span>
                        {tool.summary && (
                          <div className="text-muted-foreground pl-4">{tool.summary}</div>
                        )}
                        {tool.error && <div className="text-destructive pl-4">{tool.error}</div>}
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
