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

import { SpawnMarker, type SpawnMarkerKind } from './SpawnMarker';

import type { ChatMessageData } from '@/api/types';
import { Card } from '@/components/ui/card';
import type { TopologyWebFrame } from '@/hooks/use-topology-subscription';

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
  /** Wall-clock anchor (ms since epoch) used to interleave history-sourced
   *  turns with history-sourced user bubbles in chronological order. Live
   *  turns from the topology stream have no timestamp axis available
   *  (`TopologyEventEntry` carries no per-frame `created_at`) so they are
   *  emitted with `null` and sort to the tail of the unified list. */
  createdAt: number | null;
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
      createdAt: null,
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

// ---------------------------------------------------------------------------
// History reducer — fold persisted ChatMessage[] into TurnCardData[]
// ---------------------------------------------------------------------------

/**
 * Extract a flat text string from a `ChatMessageData.content`, which the
 * backend serialises as either a plain string or a list of multimodal
 * blocks. The timeline only renders text today, so non-text blocks are
 * dropped — matches what `buildTurnsFromEvents` does for live `text_delta`
 * frames. Exported so `TimelineView` can reuse the same flattening for
 * persisted user-message bubbles instead of duplicating it.
 */
export function contentToText(content: ChatMessageData['content']): string {
  if (typeof content === 'string') return content;
  return content
    .map((block) => (block.type === 'text' ? block.text : ''))
    .filter((s) => s.length > 0)
    .join('');
}

/**
 * Fold a persisted `ChatMessage[]` (from `GET /messages`) into the same
 * `TurnCardData` shape that the live reducer produces.
 *
 * Grouping rule: an assistant turn opens at the first assistant or tool
 * message after a user message (or at the start of the list) and closes
 * at the next user message or end-of-list. Each assistant message
 * contributes `text` and `toolCalls` (matched up with following
 * `tool_result` entries by `tool_call_id` / positional order). System
 * and user messages do NOT produce turns here — user prompts are rendered
 * separately as bubbles in `TimelineView`, and system prompts are not
 * shown in the timeline today.
 *
 * History does not carry the live metrics / usage frames, so each
 * historical turn is emitted with `inFlight: false`, `metrics: null`,
 * `usage: null`, and `markers: []`.
 */
export function buildTurnsFromHistory(messages: ChatMessageData[]): TurnCardData[] {
  const turns: TurnCardData[] = [];
  let current: TurnCardData | null = null;

  const flush = () => {
    if (current) {
      // Drop turns whose every channel is empty — the backend persists
      // tool-call slot entries with `content: ""` (e.g. seq 26/28/29 on a
      // typical assistant turn). Without this filter each empty entry
      // would render as an over-tall blank `Card` frame.
      const c = current;
      const hasContent = c.text.length > 0 || c.reasoning.length > 0 || c.toolCalls.length > 0;
      if (hasContent) turns.push(c);
      current = null;
    }
  };

  const ensure = (seedSeq: number, createdAt: number | null): TurnCardData => {
    if (current) {
      // First real timestamp wins — empty leading entries should not
      // anchor the turn's chronological position.
      if (current.createdAt === null && createdAt !== null) {
        current.createdAt = createdAt;
      }
      return current;
    }
    current = {
      id: `history-turn-${String(seedSeq)}`,
      text: '',
      reasoning: '',
      toolCalls: [],
      markers: [],
      metrics: null,
      usage: null,
      inFlight: false,
      createdAt,
    };
    return current;
  };

  const parseTs = (s: string | undefined): number | null => {
    if (!s) return null;
    const n = Date.parse(s);
    return Number.isNaN(n) ? null : n;
  };

  for (const msg of messages) {
    switch (msg.role) {
      case 'system':
      case 'user':
        // User / system messages close the prior assistant turn (if any)
        // and do not themselves produce a TurnCard.
        flush();
        break;
      case 'assistant': {
        const text = contentToText(msg.content);
        const toolCalls = msg.tool_calls ?? [];
        // Whitespace-only entries with no tool_calls are tool-call slot
        // remnants — they would otherwise prepend leading newlines to the
        // next real content (or open a turn that flush() then drops),
        // producing an empty `\n\n\n\n` band at the top of the card.
        if (text.trim().length === 0 && toolCalls.length === 0) {
          break;
        }
        const turn = ensure(msg.seq, parseTs(msg.created_at));
        if (text) {
          // Backend may emit multiple assistant tape entries within one
          // logical turn (text + a separate tool_call entry). Concatenate
          // text rather than overwriting so neither piece is lost.
          turn.text = turn.text ? `${turn.text}${text}` : text;
        }
        for (const tc of toolCalls) {
          turn.toolCalls.push({ id: tc.id, name: tc.name, result: null });
        }
        break;
      }
      case 'tool':
      case 'tool_result': {
        // Tool result messages attach back onto the open assistant turn.
        // If somehow a tool_result arrives before any assistant entry,
        // start a fresh turn anchored on its seq so the result is still
        // visible rather than dropped.
        const turn = ensure(msg.seq, parseTs(msg.created_at));
        const resultText = contentToText(msg.content);
        const matched = msg.tool_call_id
          ? turn.toolCalls.find((c) => c.id === msg.tool_call_id && c.result === null)
          : turn.toolCalls.find((c) => c.result === null);
        if (matched) {
          matched.result = { success: true, preview: resultText, error: null };
        } else if (msg.tool_call_id || msg.tool_name) {
          // No prior tool_call entry to match — synthesise a row so the
          // result is not silently dropped.
          turn.toolCalls.push({
            id: msg.tool_call_id ?? `tool-${String(msg.seq)}`,
            name: msg.tool_name ?? 'tool',
            result: { success: true, preview: resultText, error: null },
          });
        }
        break;
      }
      default:
        break;
    }
  }

  flush();
  return turns;
}
