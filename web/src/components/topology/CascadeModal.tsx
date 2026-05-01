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

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { useCascadeTrace } from '@/hooks/use-trace-fetch';

export interface CascadeModalProps {
  /** Session key whose cascade endpoint we hit. */
  sessionKey: string;
  /** Per-turn seq — same key as the execution trace; cascade is scoped to the turn. */
  seq: number | null;
  /** Whether the modal is open. */
  open: boolean;
  /** Close handler driven by the dialog's onOpenChange. */
  onOpenChange: (open: boolean) => void;
}

/**
 * Cascade (think → act → observe) modal for an assistant turn. Renders
 * the structured tick / entry breakdown returned by `GET /trace?seq=…`.
 *
 * The cascade is per-turn, not per-tool — clicking any tool activity row
 * within a turn opens the same modal, since the cascade is the right
 * granularity to inspect a turn's reasoning chain. If we later need
 * tool-scoped views, that goes on top of this without changing the wire
 * shape.
 */
export function CascadeModal({ sessionKey, seq, open, onOpenChange }: CascadeModalProps) {
  const { data, isLoading, isError, error } = useCascadeTrace(sessionKey, seq, open);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Cascade trace</DialogTitle>
          <DialogDescription>
            seq {seq ?? '—'} · session <span className="font-mono text-xs">{sessionKey}</span>
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] overflow-y-auto text-sm">
          {isLoading && <div className="text-muted-foreground">Loading…</div>}
          {isError && (
            <div role="alert" className="text-destructive">
              Failed to load cascade: {error instanceof Error ? error.message : String(error)}
            </div>
          )}
          {data && (
            <div className="space-y-3">
              <div className="text-xs text-muted-foreground">
                {data.summary.tick_count} tick(s) · {data.summary.tool_call_count} tool call(s) ·{' '}
                {data.summary.total_entries} entries
              </div>
              {data.ticks.map((tick) => (
                <section key={tick.index} className="border-l-2 border-muted pl-3">
                  <h3 className="font-medium mb-1">Tick {tick.index}</h3>
                  <ul className="space-y-1">
                    {tick.entries.map((entry) => (
                      <li key={entry.id} className="text-xs">
                        <span className="font-mono uppercase text-[10px] text-muted-foreground">
                          {entry.kind}
                        </span>{' '}
                        <span className="whitespace-pre-wrap">{entry.content}</span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
