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

import { X } from "lucide-react";
import type { DockBlock } from "@/api/dock";
import { Button } from "@/components/ui/button";

/**
 * Basic HTML sanitizer that strips dangerous patterns:
 * - <script> and <iframe> tags
 * - on* event handler attributes
 * - javascript: URLs
 */
function sanitizeHtml(html: string): string {
  return html
    .replace(/<script[\s>][\s\S]*?<\/script>/gi, "")
    .replace(/<iframe[\s>][\s\S]*?<\/iframe>/gi, "")
    .replace(/<script[\s>]/gi, "&lt;script ")
    .replace(/<iframe[\s>]/gi, "&lt;iframe ")
    .replace(/\s+on\w+\s*=\s*("[^"]*"|'[^']*'|[^\s>]*)/gi, "")
    .replace(/href\s*=\s*"javascript:[^"]*"/gi, 'href="#"')
    .replace(/href\s*=\s*'javascript:[^']*'/gi, "href='#'");
}

interface DockDiffViewProps {
  original: string;
  modified: string;
  onDismiss: () => void;
}

function DockDiffView({ original, modified, onDismiss }: DockDiffViewProps) {
  const origLines = original.split("\n");
  const modLines = modified.split("\n");

  return (
    <div className="mt-2 rounded-lg border border-border/60 bg-muted/30 text-xs font-mono overflow-hidden">
      <div className="flex items-center justify-between border-b border-border/40 px-3 py-1.5">
        <span className="text-muted-foreground text-[11px] font-medium uppercase tracking-wide">
          Diff
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="h-5 w-5"
          onClick={onDismiss}
        >
          <X className="h-3 w-3" />
        </Button>
      </div>
      <div className="p-2 space-y-0.5">
        {origLines.map((line, i) => (
          <div
            key={`rem-${i}`}
            className="rounded px-2 py-0.5 bg-destructive/10 text-destructive line-through"
          >
            - {line}
          </div>
        ))}
        {modLines.map((line, i) => (
          <div
            key={`add-${i}`}
            className="rounded px-2 py-0.5 bg-green-500/10 text-green-700 dark:text-green-400"
          >
            + {line}
          </div>
        ))}
      </div>
    </div>
  );
}

/**
 * Sanitize HTML to prevent XSS from agent-controlled content.
 * Strips script/iframe tags, event handler attributes, and javascript: URLs.
 */
function sanitizeHtml(html: string): string {
  // Remove <script> tags and their contents
  let sanitized = html.replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, "");
  // Remove <iframe> tags and their contents
  sanitized = sanitized.replace(/<iframe\b[^<]*(?:(?!<\/iframe>)<[^<]*)*<\/iframe>/gi, "");
  // Remove self-closing script/iframe tags
  sanitized = sanitized.replace(/<(?:script|iframe)\b[^>]*\/?\s*>/gi, "");
  // Remove event handler attributes (on*)
  sanitized = sanitized.replace(/\s+on\w+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi, "");
  // Remove javascript: URLs from href and src attributes
  sanitized = sanitized.replace(/(href|src)\s*=\s*(?:"javascript:[^"]*"|'javascript:[^']*')/gi, '$1=""');
  return sanitized;
}

interface DockBlockRendererProps {
  block: DockBlock;
  onDismissDiff: (id: string) => void;
}

export default function DockBlockRenderer({
  block,
  onDismissDiff,
}: DockBlockRendererProps) {
  return (
    <div className="dock-block-inner group rounded-xl border border-border/50 bg-card/60 p-4 transition-colors hover:border-border">
      <div
        className="prose prose-sm dark:prose-invert max-w-none"
        dangerouslySetInnerHTML={{ __html: sanitizeHtml(block.html) }}
      />
      {block.diff && (
        <DockDiffView
          original={block.diff.original}
          modified={block.diff.modified}
          onDismiss={() => onDismissDiff(block.id)}
        />
      )}
    </div>
  );
}
