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

/**
 * Bridges rara backend tool names to pi-mono's renderers, plus a
 * rara-specific compact renderer for tools without a specialized
 * pi-mono equivalent.
 *
 * Must be called once, BEFORE `ChatPanel.setAgent()`. The registry is a
 * module-level `Map`, so late registration would miss the first render.
 */

import { BashRenderer, registerToolRenderer } from "@mariozechner/pi-web-ui";
import { CompactToolRenderer } from "./CompactToolRenderer";

/**
 * Tool names declared by the rara backend (see `crates/app/src/tools/`).
 * `bash`, `artifacts`, `javascript_repl`, and `extract_document` keep
 * their specialized pi-mono renderers; everything else gets a compact
 * single-line renderer that collapses the Input/Output JSON behind a
 * disclosure, so tool-heavy assistant turns do not dominate the chat
 * column.
 */
const RARA_COMPACT_TOOLS = [
	"acp-delegate",
	"ask-user",
	"create-directory",
	"create-skill",
	"debug_trace",
	"delete-file",
	"delete-skill",
	"discover-tools",
	"dispatch-rara",
	"distill-user-notes",
	"edit-file",
	"evolve-soul",
	"fff-find",
	"fff-grep",
	"file-stats",
	"find-files",
	"get-session-info",
	"grep",
	"http-fetch",
	"install-acp-agent",
	"install-mcp-server",
	"list-acp-agents",
	"list-directory",
	"list-mcp-servers",
	"list-sessions",
	"list-skills",
	"multi-edit",
	"read-file",
	"read-tape",
	"remove-acp-agent",
	"remove-mcp-server",
	"send-email",
	"send-file",
	"set-avatar",
	"settings",
	"system-paths",
	"type",
	"update-session-title",
	"update-soul-state",
	"user-note",
	"walk-directory",
	"wechat-login-confirm",
	"wechat-login-start",
	"write-file",
	"write-skill-draft",
	"write-user-note",
];

export function registerRaraToolRenderers(): void {
	registerToolRenderer("bash", new BashRenderer());

	for (const name of RARA_COMPACT_TOOLS) {
		registerToolRenderer(name, new CompactToolRenderer(name));
	}
}
