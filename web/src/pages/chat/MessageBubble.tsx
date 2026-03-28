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
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Bot, User } from "lucide-react";
import type { ChatMessageData } from "@/api/types";
import { cn } from "@/lib/utils";
import { imageBlockSrc } from "@/lib/chat-attachments";
import type { TurnMetrics } from "./types";
import { extractTextContent, formatTime } from "./utils";

// ---------------------------------------------------------------------------
// MessageBubble
// ---------------------------------------------------------------------------

function ImageBlock({ src }: { src: string }) {
  const [failed, setFailed] = useState(false);

  if (failed) {
    return (
      <div className="flex h-32 w-48 items-center justify-center rounded-lg border border-dashed border-muted-foreground/30 bg-muted/30 text-xs text-muted-foreground">
        Image failed to load
      </div>
    );
  }

  return (
    <img
      src={src}
      alt=""
      className="max-h-64 max-w-xs rounded-lg object-contain"
      onError={() => setFailed(true)}
    />
  );
}

export function MessageBubble({ msg, metrics, onClick }: { msg: ChatMessageData; metrics?: TurnMetrics | null; onClick?: () => void }) {
  const isUser = msg.role === "user";
  const isSystem = msg.role === "system";
  const isMultimodal = Array.isArray(msg.content);
  const text = extractTextContent(msg.content);

  if (isSystem) {
    return (
      <div className="mx-auto max-w-md rounded-full border border-border/70 bg-background/80 px-4 py-2 text-center text-xs text-muted-foreground italic shadow-sm">
        {text}
      </div>
    );
  }

  return (
    <div
      className={cn("group flex gap-3 cursor-pointer", isUser ? "flex-row-reverse" : "flex-row")}
      onClick={() => onClick?.()}
    >
      {/* Avatar */}
      <div
        className={cn(
          "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-xs font-medium",
          isUser
            ? "bg-primary/90 text-primary-foreground"
            : "bg-background/60 text-muted-foreground",
        )}
      >
        {isUser ? <User className="h-4 w-4" /> : <Bot className="h-4 w-4" />}
      </div>

      {/* Content */}
      <div
        className={cn(
          isUser ? "max-w-[78%]" : "max-w-[min(78ch,calc(100%-4rem))] w-full",
          isUser
            ? "rounded-2xl bg-primary/90 px-4 py-2.5 text-primary-foreground"
            : "px-1 py-1 text-foreground",
        )}
      >
        {isMultimodal ? (
          <div className="space-y-2">
            {(msg.content as import("@/api/types").ChatContentBlock[]).map(
              (block, i) => {
                if (block.type === "text") {
                  return isUser ? (
                    <p key={i} className="whitespace-pre-wrap text-sm">
                      {block.text}
                    </p>
                  ) : (
                    <div
                      key={i}
                      className="prose prose-sm max-w-none text-foreground prose-p:text-foreground prose-li:text-foreground prose-strong:text-foreground prose-headings:text-foreground prose-code:text-foreground [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs"
                    >
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {block.text}
                      </ReactMarkdown>
                    </div>
                  );
                }
                if (block.type === "image_url" || block.type === "image_base64") {
                  return <ImageBlock key={i} src={imageBlockSrc(block)} />;
                }
                return null;
              },
            )}
          </div>
        ) : isUser ? (
          <p className="whitespace-pre-wrap text-sm">{text}</p>
        ) : (
          <div className="prose prose-sm max-w-none text-foreground prose-p:text-foreground prose-li:text-foreground prose-strong:text-foreground prose-headings:text-foreground prose-code:text-foreground [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
          </div>
        )}
        <p
          className={cn(
            "mt-1 text-[10px]",
            isUser ? "text-primary-foreground/70" : "text-muted-foreground",
          )}
        >
          {formatTime(msg.created_at)}
          {!isUser && metrics && (
            <span className="ml-2 opacity-0 group-hover:opacity-100 transition-opacity">
              {metrics.model.split("/").pop() ?? metrics.model} · {(metrics.duration_ms / 1000).toFixed(1)}s · {metrics.iterations} iter · {metrics.tool_calls} tools
            </span>
          )}
        </p>
      </div>
    </div>
  );
}
