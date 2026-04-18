/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

/**
 * A compact tool renderer inspired by multica's `ToolCallRow`
 * (vendor/multica/packages/views/chat/components/chat-message-list.tsx).
 *
 * Default pi-web-ui `DefaultRenderer` is visually heavy: two labeled
 * JSON code blocks (Input/Output) plus a header, per tool call. That
 * dominates the chat column when an assistant turn chains several
 * tools. This renderer compresses each call into a single-line header
 * with a short summary derived from the params, and hides the JSON
 * payloads behind a collapsible section.
 */

import type { ToolResultMessage } from "@mariozechner/pi-ai";
import { html } from "lit";
import { createRef, ref } from "lit/directives/ref.js";
import { Wrench } from "lucide";
import {
	renderCollapsibleHeader,
	renderHeader,
	type ToolRenderer,
	type ToolRenderResult,
} from "@mariozechner/pi-web-ui";

const MAX_SUMMARY = 120;
const MAX_RESULT_EXPANDED = 4000;

function shortenPath(p: string): string {
	const parts = p.split("/");
	if (parts.length <= 3) return p;
	return ".../" + parts.slice(-2).join("/");
}

function truncate(s: string, n: number): string {
	return s.length > n ? s.slice(0, n) + "…" : s;
}

/**
 * Pull a human-readable one-liner out of a tool's input JSON. Field
 * priority matches multica's `getToolSummary` so call sites feel
 * consistent for users who have seen both apps.
 */
function summarizeParams(params: unknown): string {
	if (!params || typeof params !== "object") return "";
	const inp = params as Record<string, unknown>;

	const pick = (k: string): string | null => {
		const v = inp[k];
		return typeof v === "string" && v.length > 0 ? v : null;
	};

	const command = pick("command");
	if (command) return truncate(command, MAX_SUMMARY);

	const query = pick("query") ?? pick("pattern");
	if (query) return truncate(query, MAX_SUMMARY);

	const filePath = pick("file_path") ?? pick("path");
	if (filePath) return shortenPath(filePath);

	const description = pick("description");
	if (description) return truncate(description, MAX_SUMMARY);

	const prompt = pick("prompt");
	if (prompt) return truncate(prompt, MAX_SUMMARY);

	const skill = pick("skill");
	if (skill) return skill;

	for (const v of Object.values(inp)) {
		if (typeof v === "string" && v.length > 0 && v.length < MAX_SUMMARY) {
			return v;
		}
	}
	return "";
}

function parseParams(raw: unknown): unknown {
	if (raw == null) return undefined;
	if (typeof raw === "string") {
		try {
			return JSON.parse(raw);
		} catch {
			return raw;
		}
	}
	return raw;
}

function formatJson(value: unknown): string {
	try {
		return JSON.stringify(value, null, 2);
	} catch {
		return String(value);
	}
}

function extractOutput(result: ToolResultMessage | undefined): string {
	if (!result) return "";
	const parts: string[] = [];
	for (const c of result.content ?? []) {
		if (c.type === "text") {
			parts.push((c as { text?: string }).text ?? "");
		}
	}
	return parts.join("\n");
}

export class CompactToolRenderer implements ToolRenderer {
	private readonly toolName: string;

	constructor(toolName: string) {
		this.toolName = toolName;
	}

	render(
		params: unknown,
		result: ToolResultMessage | undefined,
		isStreaming?: boolean,
	): ToolRenderResult {
		const state: "inprogress" | "complete" | "error" = result
			? result.isError
				? "error"
				: "complete"
			: isStreaming
				? "inprogress"
				: "complete";

		const parsed = parseParams(params);
		const summary = summarizeParams(parsed);
		const output = extractOutput(result);

		const headerLabel = html`
			<span class="flex items-center gap-2 min-w-0">
				<span class="font-medium text-foreground shrink-0">${this.toolName}</span>
				${
					summary
						? html`<span class="truncate text-muted-foreground">${summary}</span>`
						: ""
				}
			</span>
		`;

		// Nothing to expand — render plain header.
		if (parsed === undefined && !output) {
			return {
				content: renderHeader(state, Wrench, headerLabel),
				isCustom: false,
			};
		}

		const contentRef = createRef<HTMLElement>();
		const chevronRef = createRef<HTMLElement>();
		const paramsJson = parsed !== undefined ? formatJson(parsed) : "";
		const outputTrimmed =
			output.length > MAX_RESULT_EXPANDED
				? output.slice(0, MAX_RESULT_EXPANDED) + "\n… (truncated)"
				: output;
		return {
			content: html`
				<div>
					${renderCollapsibleHeader(state, Wrench, headerLabel, contentRef, chevronRef, false)}
					<div
						${ref(contentRef)}
						class="overflow-hidden transition-[max-height] duration-200 max-h-0"
					>
						${
							paramsJson
								? html`
									<div class="mb-2">
										<div class="text-[11px] font-medium mb-1 text-muted-foreground">Input</div>
										<pre class="max-h-40 overflow-auto rounded bg-muted/50 p-2 text-[11px] text-muted-foreground whitespace-pre-wrap break-all">${paramsJson}</pre>
									</div>
								`
								: ""
						}
						${
							output
								? html`
									<div>
										<div class="text-[11px] font-medium mb-1 text-muted-foreground">Output</div>
										<pre class="max-h-60 overflow-auto rounded bg-muted/50 p-2 text-[11px] text-muted-foreground whitespace-pre-wrap break-all">${outputTrimmed}</pre>
									</div>
								`
								: ""
						}
					</div>
				</div>
			`,
			isCustom: false,
		};
	}
}
