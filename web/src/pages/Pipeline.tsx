/*
 * Copyright 2025 Crrow
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

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type {
  PipelineRun,
  PipelineRunEvent,
  PipelineStatus,
  PipelineStreamEvent,
  RuntimeSettingsView,
} from "@/api/types";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  FileText,
  Loader2,
  Play,
  RefreshCw,
  Square,
  Wrench,
  XCircle,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatRelativeTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const now = Date.now();
  const diffMs = now - d.getTime();
  const diffSec = Math.floor(diffMs / 1000);

  if (diffSec < 60) return "just now";
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.floor(diffHr / 24);
  if (diffDay < 7) return `${diffDay}d ago`;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function formatDuration(startIso: string, endIso: string | null): string {
  const start = new Date(startIso).getTime();
  const end = endIso ? new Date(endIso).getTime() : Date.now();
  if (Number.isNaN(start)) return "--";
  const diffSec = Math.floor((end - start) / 1000);
  if (diffSec < 60) return `${diffSec}s`;
  const min = Math.floor(diffSec / 60);
  const sec = diffSec % 60;
  if (min < 60) return `${min}m ${sec}s`;
  const hr = Math.floor(min / 60);
  return `${hr}h ${min % 60}m`;
}

function statusVariant(
  status: PipelineRun["status"],
): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "Running":
      return "default";
    case "Completed":
      return "secondary";
    case "Failed":
      return "destructive";
    case "Cancelled":
      return "outline";
  }
}

function statusIcon(status: PipelineRun["status"]) {
  switch (status) {
    case "Running":
      return <Loader2 className="h-3 w-3 animate-spin" />;
    case "Completed":
      return <CheckCircle2 className="h-3 w-3" />;
    case "Failed":
      return <XCircle className="h-3 w-3" />;
    case "Cancelled":
      return <Square className="h-3 w-3" />;
  }
}

/** Infer a human-readable stage name from a tool call name. */
function inferStage(toolName: string): string {
  if (toolName === "get_job_preferences") return "Reading preferences";
  if (toolName.startsWith("score_job") || toolName === "score_job")
    return "Scoring jobs";
  if (
    [
      "prepare_resume_worktree",
      "read_resume_file",
      "write_resume_file",
      "render_resume",
      "finalize_resume",
    ].includes(toolName)
  )
    return "Optimizing resume";
  if (toolName === "send_email") return "Sending application";
  if (toolName === "notify") return "Sending notification";
  return toolName;
}

/** Parse an SSE chunk buffer into events. Returns [parsedEvents, remainingBuffer]. */
function parseSSEChunk(
  buffer: string,
): [PipelineStreamEvent[], string] {
  const events: PipelineStreamEvent[] = [];
  const parts = buffer.split("\n\n");
  const remaining = parts.pop() ?? "";

  for (const part of parts) {
    if (!part.trim()) continue;
    let data = "";
    for (const line of part.split("\n")) {
      if (line.startsWith("data:")) {
        data += line.slice(5).trim();
      }
    }
    if (data) {
      try {
        events.push(JSON.parse(data) as PipelineStreamEvent);
      } catch {
        // Ignore malformed events
      }
    }
  }
  return [events, remaining];
}

// ---------------------------------------------------------------------------
// Types for SSE stream state
// ---------------------------------------------------------------------------

interface ActiveToolCall {
  id: string;
  name: string;
  stage: string;
}

interface CompletedToolCall {
  id: string;
  name: string;
  stage: string;
  success: boolean;
  error?: string;
}

interface StreamState {
  isConnected: boolean;
  text: string;
  isThinking: boolean;
  activeTools: ActiveToolCall[];
  completedTools: CompletedToolCall[];
  iterations: number;
  error: string | null;
  done: boolean;
  summary: string | null;
}

const INITIAL_STREAM: StreamState = {
  isConnected: false,
  text: "",
  isThinking: false,
  activeTools: [],
  completedTools: [],
  iterations: 0,
  error: null,
  done: false,
  summary: null,
};

// ---------------------------------------------------------------------------
// RunDetail: expandable detail for a single pipeline run
// ---------------------------------------------------------------------------

function RunDetail({
  run,
  isPipelineRunning,
}: {
  run: PipelineRun;
  isPipelineRunning: boolean;
}) {
  const [stream, setStream] = useState<StreamState>(INITIAL_STREAM);
  const abortRef = useRef<AbortController | null>(null);

  // For historical (non-running) runs, fetch stored events
  const isLive = run.status === "Running" && isPipelineRunning;

  const eventsQuery = useQuery({
    queryKey: ["pipeline-run-events", run.id],
    queryFn: () =>
      api.get<PipelineRunEvent[]>(`/api/v1/pipeline/runs/${run.id}/events`),
    enabled: !isLive,
  });

  // SSE connection for live runs
  useEffect(() => {
    if (!isLive) return;

    const controller = new AbortController();
    abortRef.current = controller;

    setStream({ ...INITIAL_STREAM, isConnected: true });

    const connect = async () => {
      try {
        const BASE_URL = import.meta.env.VITE_API_URL || "";
        const res = await fetch(`${BASE_URL}/api/v1/pipeline/stream`, {
          signal: controller.signal,
        });

        if (!res.ok) {
          const errText = await res.text();
          setStream((s) => ({
            ...s,
            isConnected: false,
            error: errText || `HTTP ${res.status}`,
          }));
          return;
        }

        const reader = res.body?.getReader();
        if (!reader) {
          setStream((s) => ({
            ...s,
            isConnected: false,
            error: "No response body",
          }));
          return;
        }

        const decoder = new TextDecoder();
        let sseBuffer = "";

        while (true) {
          const { done, value } = await reader.read();
          if (done) break;

          sseBuffer += decoder.decode(value, { stream: true });
          const [events, remaining] = parseSSEChunk(sseBuffer);
          sseBuffer = remaining;

          for (const event of events) {
            switch (event.type) {
              case "started":
                // Pipeline started
                break;
              case "iteration":
                setStream((s) => ({ ...s, iterations: event.index + 1 }));
                break;
              case "thinking":
                setStream((s) => ({ ...s, isThinking: true }));
                break;
              case "thinking_done":
                setStream((s) => ({ ...s, isThinking: false }));
                break;
              case "text_delta":
                setStream((s) => ({ ...s, text: s.text + event.text }));
                break;
              case "tool_call_start":
                setStream((s) => ({
                  ...s,
                  activeTools: [
                    ...s.activeTools,
                    {
                      id: event.id,
                      name: event.name,
                      stage: inferStage(event.name),
                    },
                  ],
                }));
                break;
              case "tool_call_end":
                setStream((s) => ({
                  ...s,
                  activeTools: s.activeTools.filter(
                    (t) => t.id !== event.id,
                  ),
                  completedTools: [
                    ...s.completedTools,
                    {
                      id: event.id,
                      name: event.name,
                      stage: inferStage(event.name),
                      success: event.success,
                      error: event.error,
                    },
                  ],
                }));
                break;
              case "done":
                setStream((s) => ({
                  ...s,
                  isConnected: false,
                  done: true,
                  summary: event.summary,
                }));
                break;
              case "error":
                setStream((s) => ({
                  ...s,
                  isConnected: false,
                  error: event.message,
                }));
                break;
            }
          }
        }
      } catch (err) {
        if (err instanceof DOMException && err.name === "AbortError") return;
        setStream((s) => ({
          ...s,
          isConnected: false,
          error: err instanceof Error ? err.message : "Stream connection failed",
        }));
      }
    };

    void connect();

    return () => {
      controller.abort();
      abortRef.current = null;
    };
  }, [isLive]);

  // Render live SSE view
  if (isLive) {
    return (
      <div className="space-y-3 rounded-lg border bg-muted/20 p-4">
        <div className="flex items-center gap-2">
          <Loader2 className="h-4 w-4 animate-spin text-primary" />
          <span className="text-sm font-medium">Live Stream</span>
          {stream.iterations > 0 && (
            <Badge variant="outline" className="text-[10px]">
              Iteration {stream.iterations}
            </Badge>
          )}
        </div>

        {/* Active tool calls */}
        {stream.activeTools.length > 0 && (
          <div className="space-y-1">
            {stream.activeTools.map((tool) => (
              <div
                key={tool.id}
                className="flex items-center gap-2 text-sm text-muted-foreground"
              >
                <Wrench className="h-3 w-3 animate-pulse" />
                <span className="font-mono text-xs">{tool.stage}</span>
                <span className="text-xs text-muted-foreground/60">
                  ({tool.name})
                </span>
              </div>
            ))}
          </div>
        )}

        {/* Thinking indicator */}
        {stream.isThinking && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            <span>Thinking...</span>
          </div>
        )}

        {/* Completed tool calls */}
        {stream.completedTools.length > 0 && (
          <div className="space-y-1">
            <p className="text-xs font-medium text-muted-foreground">
              Completed ({stream.completedTools.length})
            </p>
            {stream.completedTools.map((tool) => (
              <div
                key={tool.id}
                className="flex items-center gap-2 text-xs"
              >
                {tool.success ? (
                  <CheckCircle2 className="h-3 w-3 text-green-500" />
                ) : (
                  <XCircle className="h-3 w-3 text-destructive" />
                )}
                <span className="font-mono">{tool.stage}</span>
                {tool.error && (
                  <span className="text-destructive">- {tool.error}</span>
                )}
              </div>
            ))}
          </div>
        )}

        {/* Streaming text */}
        {stream.text && (
          <div className="rounded border bg-background p-3">
            <p className="whitespace-pre-wrap text-sm">{stream.text}</p>
          </div>
        )}

        {/* Done summary */}
        {stream.done && stream.summary && (
          <div className="rounded border border-green-200 bg-green-50 p-3 dark:border-green-900 dark:bg-green-950">
            <p className="text-sm font-medium text-green-800 dark:text-green-200">
              {stream.summary}
            </p>
          </div>
        )}

        {/* Error */}
        {stream.error && (
          <div className="rounded border border-destructive/30 bg-destructive/10 p-3">
            <p className="text-sm text-destructive">{stream.error}</p>
          </div>
        )}
      </div>
    );
  }

  // Render historical events
  if (eventsQuery.isLoading) {
    return (
      <div className="space-y-2 p-4">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="h-4 w-36" />
        <Skeleton className="h-4 w-56" />
      </div>
    );
  }

  const events = eventsQuery.data ?? [];

  // Group events by type for display
  const toolCallStarts = events.filter((e) => e.event_type === "tool_call_start");
  const toolCallEnds = events.filter((e) => e.event_type === "tool_call_end");
  const textDeltas = events.filter((e) => e.event_type === "text_delta");
  const errorEvents = events.filter((e) => e.event_type === "error");

  // Build completed tool call pairs
  const completedCalls: {
    id: string;
    name: string;
    stage: string;
    success: boolean;
    error?: string;
  }[] = [];
  for (const start of toolCallStarts) {
    const payload = start.payload as { id?: string; name?: string };
    const id = payload.id ?? "";
    const name = payload.name ?? "";
    const end = toolCallEnds.find(
      (e) => (e.payload as { id?: string }).id === id,
    );
    const endPayload = end?.payload as {
      success?: boolean;
      error?: string;
    } | undefined;
    completedCalls.push({
      id,
      name,
      stage: inferStage(name),
      success: endPayload?.success ?? true,
      error: endPayload?.error,
    });
  }

  const accumulatedText = textDeltas
    .map((e) => (e.payload as { text?: string }).text ?? "")
    .join("");

  return (
    <div className="space-y-3 rounded-lg border bg-muted/20 p-4">
      {/* Summary */}
      {run.summary && (
        <div className="rounded border border-green-200 bg-green-50 p-3 dark:border-green-900 dark:bg-green-950">
          <p className="text-sm font-medium text-green-800 dark:text-green-200">
            {run.summary}
          </p>
        </div>
      )}

      {/* Error */}
      {run.error && (
        <div className="rounded border border-destructive/30 bg-destructive/10 p-3">
          <p className="text-sm text-destructive">{run.error}</p>
        </div>
      )}

      {/* Tool calls */}
      {completedCalls.length > 0 && (
        <div className="space-y-1">
          <p className="text-xs font-medium text-muted-foreground">
            Tool Calls ({completedCalls.length})
          </p>
          {completedCalls.map((tool) => (
            <div
              key={tool.id}
              className="flex items-center gap-2 text-xs"
            >
              {tool.success ? (
                <CheckCircle2 className="h-3 w-3 text-green-500" />
              ) : (
                <XCircle className="h-3 w-3 text-destructive" />
              )}
              <span className="font-mono">{tool.stage}</span>
              <span className="text-muted-foreground/60">({tool.name})</span>
              {tool.error && (
                <span className="text-destructive">- {tool.error}</span>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Accumulated text */}
      {accumulatedText && (
        <div className="rounded border bg-background p-3">
          <p className="whitespace-pre-wrap text-sm">{accumulatedText}</p>
        </div>
      )}

      {/* Error events */}
      {errorEvents.length > 0 && (
        <div className="space-y-1">
          {errorEvents.map((e) => (
            <div
              key={e.id}
              className="flex items-center gap-2 text-xs text-destructive"
            >
              <AlertCircle className="h-3 w-3" />
              <span>
                {(e.payload as { message?: string }).message ?? "Unknown error"}
              </span>
            </div>
          ))}
        </div>
      )}

      {events.length === 0 && !run.summary && !run.error && (
        <p className="text-sm text-muted-foreground">No events recorded for this run.</p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// RunRow: a single run in the historical runs list
// ---------------------------------------------------------------------------

function RunRow({
  run,
  isExpanded,
  onToggle,
  isPipelineRunning,
}: {
  run: PipelineRun;
  isExpanded: boolean;
  onToggle: () => void;
  isPipelineRunning: boolean;
}) {
  return (
    <div className="border-b last:border-b-0">
      <button
        type="button"
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-accent/50"
        onClick={onToggle}
      >
        {isExpanded ? (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <Badge
              variant={statusVariant(run.status)}
              className="gap-1 text-[10px]"
            >
              {statusIcon(run.status)}
              {run.status}
            </Badge>
            <span className="text-xs text-muted-foreground">
              {formatRelativeTime(run.started_at)}
            </span>
          </div>
        </div>

        {/* Stats */}
        <div className="hidden items-center gap-4 text-xs text-muted-foreground sm:flex">
          <span title="Jobs found">{run.jobs_found} found</span>
          <span title="Jobs scored">{run.jobs_scored} scored</span>
          <span title="Jobs applied">{run.jobs_applied} applied</span>
        </div>

        {/* Duration */}
        <div className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
          <Clock className="h-3 w-3" />
          <span>{formatDuration(run.started_at, run.finished_at)}</span>
        </div>
      </button>

      {isExpanded && (
        <div className="px-4 pb-4">
          {/* Stats row for mobile */}
          <div className="mb-3 flex flex-wrap gap-3 text-xs text-muted-foreground sm:hidden">
            <span>{run.jobs_found} found</span>
            <span>{run.jobs_scored} scored</span>
            <span>{run.jobs_applied} applied</span>
            <span>{run.jobs_notified} notified</span>
          </div>
          <RunDetail run={run} isPipelineRunning={isPipelineRunning} />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// PipelineConfig: read-only pipeline configuration
// ---------------------------------------------------------------------------

function PipelineConfig() {
  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.get<RuntimeSettingsView>("/api/v1/settings"),
  });

  const [collapsed, setCollapsed] = useState(true);

  if (!settingsQuery.data?.job_pipeline) return null;

  const pipeline = settingsQuery.data.job_pipeline;

  return (
    <Card>
      <CardHeader className="cursor-pointer" onClick={() => setCollapsed((c) => !c)}>
        <div className="flex items-center justify-between">
          <div className="space-y-1">
            <CardTitle className="flex items-center gap-2 text-base">
              <FileText className="h-4 w-4" />
              Pipeline Configuration
            </CardTitle>
            <CardDescription>
              Score thresholds and resume path.
            </CardDescription>
          </div>
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className="text-[10px]">
              Read-only
            </Badge>
            {collapsed ? (
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            ) : (
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
        </div>
      </CardHeader>
      {!collapsed && (
        <CardContent className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1 rounded-lg border bg-muted/30 p-3">
              <p className="text-xs font-medium text-muted-foreground">
                Auto-Apply Threshold
              </p>
              <p className="text-2xl font-bold">
                {pipeline.score_threshold_auto}
              </p>
              <p className="text-xs text-muted-foreground">
                Jobs scoring above this are auto-applied.
              </p>
            </div>
            <div className="space-y-1 rounded-lg border bg-muted/30 p-3">
              <p className="text-xs font-medium text-muted-foreground">
                Notify Threshold
              </p>
              <p className="text-2xl font-bold">
                {pipeline.score_threshold_notify}
              </p>
              <p className="text-xs text-muted-foreground">
                Jobs scoring above this trigger a notification.
              </p>
            </div>
          </div>
          {pipeline.resume_project_path && (
            <div className="space-y-1 rounded-lg border bg-muted/30 p-3">
              <p className="text-xs font-medium text-muted-foreground">
                Resume Project Path
              </p>
              <p className="break-all font-mono text-sm">
                {pipeline.resume_project_path}
              </p>
            </div>
          )}
          <Separator />
          <div className="space-y-2">
            <p className="text-sm font-semibold">Job Preferences</p>
            {pipeline.job_preferences ? (
              <>
                <div className="rounded-lg border bg-muted/30 p-4">
                  <div className="whitespace-pre-wrap font-mono text-sm leading-relaxed">
                    {pipeline.job_preferences}
                  </div>
                </div>
                <p className="text-xs text-muted-foreground">
                  Modify via chat with rara.
                </p>
              </>
            ) : (
              <div className="rounded-lg border border-dashed bg-muted/10 p-4 text-center">
                <p className="text-sm text-muted-foreground">
                  Not configured
                </p>
                <p className="mt-1 text-xs text-muted-foreground">
                  Tell rara about your ideal role, tech stack, location, and salary
                  expectations to set up job preferences.
                </p>
              </div>
            )}
          </div>
        </CardContent>
      )}
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Pipeline (main page component)
// ---------------------------------------------------------------------------

export default function Pipeline() {
  const queryClient = useQueryClient();
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);
  const [toast, setToast] = useState<{
    kind: "success" | "error";
    message: string;
  } | null>(null);

  // Auto-dismiss toast
  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 3000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  // Pipeline status
  const statusQuery = useQuery({
    queryKey: ["pipeline-status"],
    queryFn: () => api.get<PipelineStatus>("/api/v1/pipeline/status"),
    refetchInterval: (query) => (query.state.data?.running ? 5000 : false),
  });

  const isPipelineRunning = statusQuery.data?.running ?? false;

  // Pipeline runs
  const runsQuery = useQuery({
    queryKey: ["pipeline-runs"],
    queryFn: () =>
      api.get<PipelineRun[]>("/api/v1/pipeline/runs?limit=20&offset=0"),
    refetchInterval: isPipelineRunning ? 10000 : false,
  });

  const runs = runsQuery.data ?? [];

  // Run pipeline
  const runMutation = useMutation({
    mutationFn: () => api.post<void>("/api/v1/pipeline/run"),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pipeline-status"] });
      queryClient.invalidateQueries({ queryKey: ["pipeline-runs"] });
      setToast({ kind: "success", message: "Pipeline started." });
    },
    onError: (e: unknown) => {
      if (e instanceof Error && "status" in e) {
        const apiErr = e as Error & { status: number };
        if (apiErr.status === 409) {
          setToast({
            kind: "error",
            message: "Pipeline is already running.",
          });
          return;
        }
        if (apiErr.status === 412) {
          setToast({
            kind: "error",
            message:
              "AI is not configured. Set up OpenRouter API key in Settings first.",
          });
          return;
        }
      }
      const message =
        e instanceof Error ? e.message : "Failed to start pipeline";
      setToast({ kind: "error", message });
    },
  });

  // Cancel pipeline
  const cancelMutation = useMutation({
    mutationFn: () => api.post<void>("/api/v1/pipeline/cancel"),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pipeline-status"] });
      queryClient.invalidateQueries({ queryKey: ["pipeline-runs"] });
      setToast({ kind: "success", message: "Pipeline cancelled." });
    },
    onError: (e: unknown) => {
      const message =
        e instanceof Error ? e.message : "Failed to cancel pipeline";
      setToast({ kind: "error", message });
    },
  });

  const handleToggleRun = useCallback(
    (id: string) => {
      setExpandedRunId((prev) => (prev === id ? null : id));
    },
    [],
  );

  // Auto-expand the running run
  useEffect(() => {
    if (isPipelineRunning && runs.length > 0) {
      const runningRun = runs.find((r) => r.status === "Running");
      if (runningRun && expandedRunId !== runningRun.id) {
        setExpandedRunId(runningRun.id);
      }
    }
  }, [isPipelineRunning, runs, expandedRunId]);

  return (
    <div className="space-y-6">
      {/* ── Top Action Bar ───────────────────────────────────── */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          {isPipelineRunning ? (
            <Button
              variant="destructive"
              onClick={() => cancelMutation.mutate()}
              disabled={cancelMutation.isPending}
            >
              {cancelMutation.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Square className="mr-2 h-4 w-4" />
              )}
              Cancel Pipeline
            </Button>
          ) : (
            <Button
              onClick={() => runMutation.mutate()}
              disabled={runMutation.isPending}
            >
              {runMutation.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Play className="mr-2 h-4 w-4" />
              )}
              Run Pipeline
            </Button>
          )}
          <Badge variant={isPipelineRunning ? "default" : "secondary"}>
            {isPipelineRunning ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3 w-3 animate-spin" />
                Running
              </span>
            ) : (
              "Idle"
            )}
          </Badge>
        </div>
        <Button
          variant="outline"
          size="icon"
          onClick={() => {
            runsQuery.refetch();
            statusQuery.refetch();
          }}
          disabled={runsQuery.isFetching || statusQuery.isFetching}
          title="Refresh"
        >
          <RefreshCw
            className={`h-4 w-4 ${
              runsQuery.isFetching || statusQuery.isFetching
                ? "animate-spin"
                : ""
            }`}
          />
        </Button>
      </div>

      {/* ── Historical Runs ──────────────────────────────────── */}
      <Card>
        <CardHeader>
          <CardTitle>Pipeline Runs</CardTitle>
          <CardDescription>
            Recent pipeline executions and their results.
          </CardDescription>
        </CardHeader>
        <CardContent className="p-0">
          {runsQuery.isLoading && (
            <div className="space-y-3 p-4">
              {Array.from({ length: 3 }).map((_, i) => (
                <Skeleton key={i} className="h-12 w-full" />
              ))}
            </div>
          )}

          {!runsQuery.isLoading && runs.length === 0 && (
            <div className="px-4 py-8 text-center text-sm text-muted-foreground">
              No pipeline runs yet. Click &quot;Run Pipeline&quot; to start the
              first run.
            </div>
          )}

          {!runsQuery.isLoading && runs.length > 0 && (
            <div>
              {runs.map((run) => (
                <RunRow
                  key={run.id}
                  run={run}
                  isExpanded={expandedRunId === run.id}
                  onToggle={() => handleToggleRun(run.id)}
                  isPipelineRunning={isPipelineRunning}
                />
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {/* ── Pipeline Configuration ───────────────────────────── */}
      <PipelineConfig />

      {/* ── Toast ────────────────────────────────────────────── */}
      {toast && (
        <div className="fixed right-6 top-6 z-50">
          <div
            className={`rounded-md border px-4 py-3 text-sm shadow-lg ${
              toast.kind === "success"
                ? "border-green-200 bg-green-50 text-green-800"
                : "border-red-200 bg-red-50 text-red-800"
            }`}
          >
            {toast.message}
          </div>
        </div>
      )}
    </div>
  );
}
