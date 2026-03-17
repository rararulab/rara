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

import DOMPurify from "dompurify";
import { X } from "lucide-react";
import type { DockBlock } from "@/api/dock";
import { Button } from "@/components/ui/button";

/**
 * Sanitize HTML using DOMPurify with an explicit allowlist.
 * Dock block content originates from the agent and is untrusted input.
 */
function sanitizeHtml(html: string): string {
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: [
      "h1", "h2", "h3", "h4", "h5", "h6",
      "p", "br", "hr", "blockquote",
      "ul", "ol", "li",
      "strong", "em", "b", "i", "u", "s", "del", "ins",
      "code", "pre", "kbd", "var", "samp",
      "a", "img",
      "table", "thead", "tbody", "tr", "th", "td",
      "div", "span", "figure", "figcaption",
      "chart",
    ],
    ALLOWED_ATTR: [
      "href", "src", "alt", "title", "class", "id",
      "width", "height", "colspan", "rowspan",
      // chart-specific attributes
      "type", "labels", "values",
    ],
  });
}

// ── Chart block parsing & rendering ──────────────────────────────────

interface ChartData {
  title: string;
  labels: string[];
  values: number[];
  type: string;
}

/**
 * Extract attribute value from an HTML tag string.
 */
function extractAttr(tag: string, name: string): string {
  const re = new RegExp(`${name}\\s*=\\s*"([^"]*)"`, "i");
  const match = tag.match(re);
  return match?.[1] ?? "";
}

/**
 * Detect whether a block contains a `<chart>` tag and parse its attributes.
 * Returns `null` when no chart data is found.
 */
function parseChart(block: DockBlock): ChartData | null {
  if (block.block_type === "chart" || /<chart[\s>]/i.test(block.html)) {
    const tagMatch = block.html.match(/<chart[^>]*>/i);
    if (!tagMatch) return null;
    const tag = tagMatch[0];

    const title = extractAttr(tag, "title");
    const labelsRaw = extractAttr(tag, "labels");
    const valuesRaw = extractAttr(tag, "values");
    const type = extractAttr(tag, "type") || "bar";

    const labels = labelsRaw ? labelsRaw.split(",").map((s) => s.trim()) : [];
    const values = valuesRaw
      ? valuesRaw.split(",").map((s) => Number(s.trim()))
      : [];

    return { title, labels, values, type };
  }
  return null;
}

function DockChart({ data }: { data: ChartData }) {
  const maxValue = Math.max(...data.values, 1);

  if (data.labels.length === 0 || data.values.length === 0) {
    return (
      <div className="rounded-lg bg-muted/30 p-4 text-xs text-muted-foreground">
        {data.title && (
          <h6 className="mb-1 font-semibold text-foreground">{data.title}</h6>
        )}
        <span>n/a</span>
      </div>
    );
  }

  return (
    <div className="rounded-lg bg-muted/30 p-4 text-xs">
      {data.title && (
        <h6 className="mb-1 font-semibold text-foreground">{data.title}</h6>
      )}
      {data.type !== "bar" && (
        <p className="mb-2 text-[11px] text-muted-foreground">
          Displayed as bar ({data.type} not supported)
        </p>
      )}
      <div className="space-y-1">
        {data.labels.map((label, i) => {
          const value = data.values[i] ?? 0;
          const pct = Math.max((value / maxValue) * 100, 2);
          return (
            <div key={i} className="flex items-center gap-2">
              <span className="w-16 shrink-0 truncate text-right text-muted-foreground">
                {label}
              </span>
              <div className="relative flex-1">
                <div
                  className="h-5 rounded bg-primary/80 transition-all"
                  style={{ width: `${pct}%` }}
                />
                <span className="absolute inset-y-0 left-1.5 flex items-center text-[11px] font-medium text-primary-foreground">
                  {value}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
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

interface DockBlockRendererProps {
  block: DockBlock;
  onDismissDiff: (id: string) => void;
}

export default function DockBlockRenderer({
  block,
  onDismissDiff,
}: DockBlockRendererProps) {
  const chartData = parseChart(block);

  return (
    <div className="dock-block-inner group rounded-xl border border-border/50 bg-card/60 p-4 transition-colors hover:border-border">
      {chartData ? (
        <DockChart data={chartData} />
      ) : (
        <div
          className="prose prose-sm dark:prose-invert max-w-none"
          dangerouslySetInnerHTML={{ __html: sanitizeHtml(block.html) }}
        />
      )}
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
