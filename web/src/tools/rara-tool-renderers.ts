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
 * Bridges rara backend tool names to pi-mono's built-in renderers.
 *
 * Rara's backend tools (declared in `crates/app/src/tools/`) emit names like
 * `bash`, `http-fetch`, `read-file`, etc. pi-mono ships tool-specific
 * renderers (`BashRenderer`, `javascript_repl`, `extract_document`,
 * `artifacts`) keyed by exact tool name. When a name does not match, the UI
 * falls back to `DefaultRenderer`, which prints raw JSON.
 *
 * This module registers aliases so any rara tool that conceptually matches a
 * pi-mono renderer gets the richer UX. Tools without a pi-mono equivalent
 * (`mita-*`, `skill-*`, `tape-*`, `marketplace-*`, ...) stay on the default
 * JSON renderer by design — writing custom renderers for them is out of
 * scope for this phase.
 *
 * Must be called once, BEFORE `ChatPanel.setAgent()`. The registry is a
 * module-level `Map`, so late registration would miss the first render.
 */

import {
	BashRenderer,
	registerToolRenderer,
} from "@mariozechner/pi-web-ui";

/**
 * Register rara → pi-mono renderer aliases.
 *
 * Current mappings:
 * - `bash` → `BashRenderer` (rara's bash tool is already named `bash`; we
 *   register explicitly so the wiring is visible and robust against any
 *   future change to pi-web-ui's auto-registration side effects).
 *
 * Left on `DefaultRenderer` intentionally:
 * - `http-fetch` — no pi-mono equivalent (extract_document is PDF/DOCX,
 *   not generic HTTP).
 * - `read-file` / `write-file` / `edit-file` / `multi-edit` / `grep` /
 *   `find-files` / `list-directory` / `walk-directory` / `file-stats` /
 *   `create-directory` / `delete-file` — pi-mono has no file-IO renderers.
 * - `mita-*`, `skill-*`, `tape-*`, `marketplace-*`, `mcp-*`, `acp-*`,
 *   `dispatch-rara`, `evolve-soul`, `ask-user`, etc. — rara-specific; JSON
 *   is acceptable until a custom renderer is justified.
 * - `javascript_repl`, `extract_document`, `artifacts` — pi-mono registers
 *   these itself; no rara-side equivalent exists to alias.
 */
export function registerRaraToolRenderers(): void {
	// `bash` is the canonical name on both sides, but we call this explicitly
	// so the binding is documented and not dependent on import-order side
	// effects inside pi-web-ui.
	registerToolRenderer("bash", new BashRenderer());
}
