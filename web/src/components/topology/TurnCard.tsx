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

import { Activity, Clock, Hammer, Loader2 } from 'lucide-react';

import { Card } from '@/components/ui/card';
import type { TopologyWebFrame } from '@/hooks/use-topology-subscription';

import { SpawnMarker, type SpawnMarkerKind } from './SpawnMarker';

/**
 * A single agent turn rendered as a card. A turn starts at the first
 * frame after the previous `done` (or at the start of the stream) and
 * extends through the next `done` frame inclusive. While a turn is
 * still in flight, `inFlight` is true and a spinner is shown alongside
 * the metrics row.
 */
export interface TurnCardData {
  /** Stable id derived from the seq of the first event in the turn. */
  id: string;
  /** Streaming assistant text accumulated from `text_delta` frames. */
  text: string;
  /** Streaming reasoning text accumulated from `reasoning_delta` frames. */
  reasoning: string;
  /** Tool calls executed during this turn in arrival order. */
  toolCalls: TurnToolCall[];
  /** Inline topology markers (spawn / done / fork) interleaved with the turn. */
  markers: SpawnMarkerKind[];
  /** Final metrics frame for this turn, if observed. */
  metrics: TurnMetrics | null;
  /** Token usage frame for this turn, if observed. */
  usage: TurnUsage | null;
  /** Whether the turn is still streaming (no terminal `done` yet). */
  inFlight: boolean;
}

export interface TurnToolCall {
  id: string;
  name: string;
  /** Filled in when `tool_call_end` arrives; undefined while pending. */
  result: { success: boolean; preview: string; error: string | null } | null;
}

export interface TurnMetrics {
  durationMs: number;
  iterations: number;
  toolCalls: number;
  model: string;
}

export interface TurnUsage {
  input: number;
  output: number;
  totalTokens: number;
  cost: number;
}

export interface TurnCardProps {
  turn: TurnCardData;
}

export function TurnCard({ turn }: TurnCardProps) {
  return (
    <Card className="space-y-3 p-4">
      {turn.reasoning && (
        <div className="rounded border-l-2 border-muted bg-muted/30 px-3 py-2 text-xs italic text-muted-foreground">
          {turn.reasoning}
        </div>
      )}

      {turn.text && (
        <div className="whitespace-pre-wrap text-sm leading-relaxed text-foreground">
          {turn.text}
        </div>
      )}

      {turn.toolCalls.length > 0 && (
        <div className="space-y-1.5">
          {turn.toolCalls.map((call) => (
            <ToolCallRow key={call.id} call={call} />
          ))}
        </div>
      )}

      {turn.markers.length > 0 && (
        <div className="space-y-1.5">
          {turn.markers.map((marker, idx) => (
            <SpawnMarker key={`${turn.id}-marker-${String(idx)}`} marker={marker} />
          ))}
        </div>
      )}

      <TurnFooter turn={turn} />
    </Card>
  );
}

function ToolCallRow({ call }: { call: TurnToolCall }) {
  const pending = call.result === null;
  const failed = call.result !== null && !call.result.success;
  return (
    <div className="flex items-start gap-2 rounded border bg-muted/20 px-2.5 py-1.5 text-xs">
      <Hammer className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="font-mono font-medium text-foreground">{call.name}</span>
          {pending && (
            <span className="flex items-center gap-1 text-muted-foreground">
              <Loader2 className="h-3 w-3 animate-spin" />
              running
            </span>
          )}
          {failed && <span className="text-red-600 dark:text-red-400">failed</span>}
        </div>
        {call.result && (
          <div className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
            {call.result.error ?? call.result.preview}
          </div>
        )}
      </div>
    </div>
  );
}

function TurnFooter({ turn }: { turn: TurnCardData }) {
  if (turn.inFlight) {
    return (
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Loader2 className="h-3 w-3 animate-spin" />
        <span>thinking…</span>
      </div>
    );
  }

  if (!turn.metrics && !turn.usage) {
    return null;
  }

  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t pt-2 text-[11px] text-muted-foreground">
      {turn.metrics && (
        <>
          <span className="flex items-center gap-1">
            <Clock className="h-3 w-3" />
            {(turn.metrics.durationMs / 1000).toFixed(1)}s
          </span>
          <span className="flex items-center gap-1">
            <Activity className="h-3 w-3" />
            {turn.metrics.iterations} iter · {turn.metrics.toolCalls} tools
          </span>
          <span className="font-mono">{turn.metrics.model}</span>
        </>
      )}
      {turn.usage && (
        <span className="ml-auto font-mono">
          {turn.usage.totalTokens.toLocaleString()} tok · ${turn.usage.cost.toFixed(4)}
        </span>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Reducer — fold a flat WebFrame stream into TurnCardData[]
// ---------------------------------------------------------------------------

/**
 * Fold every event observed on the root session into a list of turn
 * cards. Each `done` frame closes a turn; subsequent frames open the
 * next one. Events from descendant sessions are NOT included here —
 * the caller is responsible for filtering before invoking.
 *
 * Frames the timeline does not currently visualize (e.g. `phase`,
 * `progress`, `attachment`, `approval_*`, `tape_appended`) are dropped
 * so the card stays focused on assistant output + tool calls + topology
 * transitions. They can be wired in later without touching the reducer
 * shape.
 */
export function buildTurnsFromEvents(
  events: { seq: number; event: TopologyWebFrame }[],
): TurnCardData[] {
  const turns: TurnCardData[] = [];
  let current: TurnCardData | null = null;

  const ensure = (seq: number): TurnCardData => {
    if (current) return current;
    current = {
      id: `turn-${String(seq)}`,
      text: '',
      reasoning: '',
      toolCalls: [],
      markers: [],
      metrics: null,
      usage: null,
      inFlight: true,
    };
    return current;
  };

  for (const { seq, event: frame } of events) {
    switch (frame.type) {
      case 'text_delta': {
        const turn = ensure(seq);
        turn.text += frame.text;
        break;
      }
      case 'reasoning_delta': {
        const turn = ensure(seq);
        turn.reasoning += frame.text;
        break;
      }
      case 'text_clear': {
        const turn = ensure(seq);
        turn.text = '';
        break;
      }
      case 'tool_call_start': {
        const turn = ensure(seq);
        turn.toolCalls.push({ id: frame.id, name: frame.name, result: null });
        break;
      }
      case 'tool_call_end': {
        const turn = ensure(seq);
        const call = turn.toolCalls.find((c) => c.id === frame.id);
        if (call) {
          call.result = {
            success: frame.success,
            preview: frame.result_preview,
            error: frame.error,
          };
        }
        break;
      }
      case 'subagent_spawned': {
        const turn = ensure(seq);
        turn.markers.push({
          kind: 'spawned',
          childSession: frame.child_session,
          manifestName: frame.manifest_name,
        });
        break;
      }
      case 'subagent_done': {
        const turn = ensure(seq);
        turn.markers.push({
          kind: 'done',
          childSession: frame.child_session,
          success: frame.success,
        });
        break;
      }
      case 'tape_forked': {
        const turn = ensure(seq);
        turn.markers.push({
          kind: 'forked',
          forkedFrom: frame.forked_from,
          childTape: frame.child_tape,
          anchor: frame.forked_at_anchor ?? null,
        });
        break;
      }
      case 'turn_metrics': {
        const turn = ensure(seq);
        turn.metrics = {
          durationMs: frame.duration_ms,
          iterations: frame.iterations,
          toolCalls: frame.tool_calls,
          model: frame.model,
        };
        break;
      }
      case 'usage': {
        const turn = ensure(seq);
        turn.usage = {
          input: frame.input,
          output: frame.output,
          totalTokens: frame.total_tokens,
          cost: frame.cost,
        };
        break;
      }
      case 'done': {
        // `ensure` mutates `current` from a closure, which TS control-flow
        // analysis can't see — without the cast, TS narrows `current` to
        // `null` after the prior-iteration assignment below and then to
        // `never` inside the truthy branch.
        const turn = current as TurnCardData | null;
        if (turn) {
          turn.inFlight = false;
          turns.push(turn);
          current = null;
        }
        break;
      }
      default:
        break;
    }
  }

  if (current) {
    turns.push(current);
  }

  return turns;
}
