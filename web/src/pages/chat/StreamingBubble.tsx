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

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { Bot, Loader2, Wrench } from "lucide-react";
import {
  formatCompletedToolLine,
  formatLiveReasoning,
  formatToolCallSummary,
} from "@/lib/chat-progress";
import type { StreamState } from "./types";

// ---------------------------------------------------------------------------
// ActivityTree — real-time tool call trace
// ---------------------------------------------------------------------------

export function ActivityTree({ stream }: { stream: StreamState }) {
  const liveReasoning = formatLiveReasoning(stream.reasoning);

  if (
    stream.activeTools.length === 0 &&
    stream.completedTools.length === 0 &&
    !liveReasoning
  ) {
    return null;
  }
  return (
    <div className="mb-2 rounded-lg border border-border/50 bg-muted/30 px-3 py-2 text-xs font-mono text-muted-foreground space-y-1">
      {liveReasoning && (
        <div className="rounded-md border border-border/40 bg-background/60 px-2 py-1.5">
          <div className="mb-1 text-[10px] uppercase tracking-[0.2em] text-muted-foreground/60">
            Thinking
          </div>
          <div className="font-sans text-xs leading-5 text-foreground/85">
            {liveReasoning}
          </div>
        </div>
      )}
      {stream.turnRationale && (
        <div className="text-muted-foreground/70 text-[11px] leading-4" aria-label="LLM reasoning">
          💭 {stream.turnRationale}
        </div>
      )}
      {stream.completedTools.map((t) => (
        <div key={t.id}>
          <div className="flex items-start gap-1.5">
            <span className="break-words">
              {formatCompletedToolLine(t)}
            </span>
          </div>
        </div>
      ))}
      {stream.activeTools.map((t) => (
        <div key={t.id}>
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-1.5 w-1.5 animate-spin rounded-full border border-blue-500 border-t-transparent" />
            <span className="break-words">
              {formatToolCallSummary(t.name, t.arguments)}
            </span>
          </div>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// StreamingBubble — live assistant response during SSE streaming
// ---------------------------------------------------------------------------

export function StreamingBubble({ stream }: { stream: StreamState }) {
  const liveReasoning = formatLiveReasoning(stream.reasoning);

  return (
    <div className="flex gap-3">
      <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-background/60 text-muted-foreground">
        <Bot className="h-4 w-4" />
      </div>
      <div className="w-full max-w-[min(78ch,calc(100%-4rem))] px-1 py-1 text-foreground">
        {liveReasoning && (
          <div className="mb-3 rounded-2xl border border-border/50 bg-background/55 px-3 py-2">
            <div className="mb-1 text-[10px] uppercase tracking-[0.2em] text-muted-foreground/60">
              Thinking
            </div>
            <p className="text-sm leading-6 text-foreground/85">
              {liveReasoning}
            </p>
          </div>
        )}

        {/* Tool call indicators */}
        {stream.activeTools.length > 0 && (
          <div className="mb-2 space-y-1">
            {stream.activeTools.map((tool) => (
              <div
                key={tool.id}
                className="flex items-center gap-1.5 text-xs text-muted-foreground"
              >
                <Wrench className="h-3 w-3 animate-pulse" />
                <span className="font-mono">
                  {formatToolCallSummary(tool.name, tool.arguments)}
                </span>
              </div>
            ))}
          </div>
        )}

        {/* Thinking indicator */}
        {stream.isThinking &&
          !stream.text &&
          !liveReasoning &&
          stream.activeTools.length === 0 && (
          <div className="flex items-center gap-2">
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            <span className="text-sm text-muted-foreground">Thinking...</span>
          </div>
          )}

        {/* Streaming text content */}
        {stream.text && (
          <div className="prose prose-sm max-w-none text-foreground prose-p:text-foreground prose-li:text-foreground prose-strong:text-foreground prose-headings:text-foreground prose-code:text-foreground [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs">
            <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeHighlight]}>
              {stream.text}
            </ReactMarkdown>
          </div>
        )}

        {/* Error */}
        {stream.error && (
          <p className="text-sm text-destructive">{stream.error}</p>
        )}
      </div>
    </div>
  );
}
