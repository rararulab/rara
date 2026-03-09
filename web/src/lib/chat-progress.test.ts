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

import { describe, expect, it } from "vitest";

import {
  formatCompletedToolLine,
  formatLiveReasoning,
  formatToolCallSummary,
} from "./chat-progress";

describe("chat progress formatting", () => {
  it("surfaces live reasoning as a readable status line", () => {
    expect(
      formatLiveReasoning("先检查项目结构，然后对比现有的日志展示方式。"),
    ).toBe("先检查项目结构，然后对比现有的日志展示方式。");
  });

  it("formats tool calls with a concrete summary instead of argument keys", () => {
    expect(
      formatToolCallSummary("read-file", {
        path: "/Users/ryan/code/rararulab/rara/web/src/pages/Chat.tsx",
      }),
    ).toBe("read-file /Users/ryan/code/rararulab/rara/web/src/pages/Chat.tsx");
  });

  it("includes the result preview for completed tools", () => {
    expect(
      formatCompletedToolLine({
        name: "read-file",
        success: true,
        result_preview: "function ActivityTree({ stream }: { stream: StreamState }) {",
        error: null,
      }),
    ).toBe(
      "\u2713 read-file function ActivityTree({ stream }: { stream: StreamState }) {",
    );
  });
});
