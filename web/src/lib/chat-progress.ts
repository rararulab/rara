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

const MAX_REASONING_CHARS = 180;
const MAX_SUMMARY_CHARS = 100;

type ToolArguments = Record<string, unknown>;

interface CompletedToolLike {
  name: string;
  success: boolean;
  result_preview: string;
  error: string | null;
}

function truncate(text: string, maxChars: number): string {
  const normalized = text.trim().replace(/\s+/g, " ");
  if (normalized.length <= maxChars) {
    return normalized;
  }
  return `${normalized.slice(0, maxChars - 1).trimEnd()}…`;
}

function firstStringValue(arguments_: ToolArguments): string | null {
  for (const value of Object.values(arguments_)) {
    if (typeof value === "string" && value.trim()) {
      return value;
    }
  }
  return null;
}

function summarizeToolArgument(name: string, arguments_: ToolArguments): string {
  const value = (() => {
    switch (name) {
      case "read-file":
      case "write-file":
        return typeof arguments_.path === "string" ? arguments_.path : null;
      case "shell_execute":
        return typeof arguments_.command === "string" ? arguments_.command : null;
      case "web_search":
        return typeof arguments_.query === "string" ? arguments_.query : null;
      case "web_fetch":
        return typeof arguments_.url === "string" ? arguments_.url : null;
      default:
        return (
          (typeof arguments_.query === "string" && arguments_.query) ||
          (typeof arguments_.command === "string" && arguments_.command) ||
          (typeof arguments_.input === "string" && arguments_.input) ||
          (typeof arguments_.path === "string" && arguments_.path) ||
          (typeof arguments_.url === "string" && arguments_.url) ||
          firstStringValue(arguments_)
        );
    }
  })();

  return value ? truncate(value, MAX_SUMMARY_CHARS) : "";
}

export function formatLiveReasoning(reasoning: string): string {
  return truncate(reasoning, MAX_REASONING_CHARS);
}

export function formatToolCallSummary(
  name: string,
  arguments_: ToolArguments,
): string {
  const summary = summarizeToolArgument(name, arguments_);
  return summary ? `${name} ${summary}` : name;
}

export function formatCompletedToolLine(tool: CompletedToolLike): string {
  const parts = [tool.success ? "\u2713" : "\u2717", tool.name];
  const preview = truncate(tool.result_preview, MAX_SUMMARY_CHARS);
  if (preview) {
    parts.push(preview);
  }
  if (tool.error) {
    parts.push(truncate(tool.error, MAX_SUMMARY_CHARS));
  }
  return parts.join(" ");
}
