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

import { useState, useEffect, Fragment } from "react";
import { useQuery } from "@tanstack/react-query";
import CascadeViewer, { type StreamingNode, type TurnTrace } from "@/components/CascadeViewer";
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Clock,
  Cpu,
  Hash,
  RefreshCw,
  Zap,
} from "lucide-react";
import { api } from "@/api/client";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

// ---------------------------------------------------------------------------
// Types (matching Rust backend)
// ---------------------------------------------------------------------------

interface SystemStats {
  active_processes: number;
  total_spawned: number;
  total_completed: number;
  total_failed: number;
  global_semaphore_available: number;
  total_tokens_consumed: number;
  uptime_ms: number;
}

interface ProcessStats {
  agent_id: string;
  session_id: string;
  manifest_name: string;
  state: string;
  parent_id: string | null;
  children: string[];
  created_at: string;
  finished_at: string | null;
  uptime_ms: number;
  messages_received: number;
  llm_calls: number;
  tool_calls: number;
  tokens_consumed: number;
  last_activity: string | null;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatUptime(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const hours = Math.floor(totalSec / 3600);
  const minutes = Math.floor((totalSec % 3600) / 60);
  const seconds = totalSec % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function formatRelativeTime(iso: string | null): string {
  if (!iso) return "-";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const diffMs = Date.now() - d.getTime();
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 5) return "just now";
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  return `${diffHr}h ago`;
}

function stateColor(state: string): "default" | "secondary" | "destructive" | "outline" {
  switch (state.toLowerCase()) {
    case "running":
      return "default";
    case "idle":
    case "paused":
      return "secondary";
    case "waiting":
      return "outline";
    case "failed":
    case "error":
      return "destructive";
    case "completed":
    case "cancelled":
      return "outline";
    default:
      return "outline";
  }
}

// ---------------------------------------------------------------------------
// useProcessStream hook
// ---------------------------------------------------------------------------

function useProcessStream(agentId: string | null, processState: string | null) {
  const [nodes, setNodes] = useState<StreamingNode[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);

  useEffect(() => {
    if (!agentId || !processState) return;
    if (processState !== "Running" && processState !== "Idle") return;

    const host = window.location.host;
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const token = localStorage.getItem("access_token") ?? "";
    const ws = new WebSocket(
      `${protocol}//${host}/api/v1/kernel/processes/${agentId}/stream?token=${token}`
    );

    let currentThought = "";
    let thoughtKey = `live-thought-${Date.now()}`;

    ws.onopen = () => {
      setIsStreaming(true);
      setNodes([]);
    };

    ws.onmessage = (ev) => {
      try {
        const event = JSON.parse(ev.data);
        switch (event.type) {
          case "text_delta":
            currentThought += event.text ?? "";
            setNodes((prev) => {
              const existing = prev.find((n) => n.key === thoughtKey);
              if (existing) {
                return prev.map((n) =>
                  n.key === thoughtKey ? { ...n, text: currentThought } : n
                );
              }
              return [
                ...prev,
                {
                  type: "thought" as const,
                  key: thoughtKey,
                  text: currentThought,
                  streaming: true,
                },
              ];
            });
            break;

          case "tool_call_start":
            if (currentThought) {
              setNodes((prev) =>
                prev.map((n) =>
                  n.key === thoughtKey ? { ...n, streaming: false } : n
                )
              );
            }
            setNodes((prev) => [
              ...prev,
              {
                type: "action" as const,
                key: `live-action-${event.id}`,
                toolName: event.name,
                toolId: event.id,
                arguments: event.arguments,
                streaming: true,
              },
            ]);
            break;

          case "tool_call_end":
            setNodes((prev) => [
              ...prev.map((n) =>
                n.key === `live-action-${event.id}`
                  ? { ...n, streaming: false, success: event.success }
                  : n
              ),
              {
                type: "observation" as const,
                key: `live-obs-${event.id}`,
                resultPreview: event.result_preview,
                success: event.success,
                error: event.error,
              },
            ]);
            break;

          case "turn_metrics":
            currentThought = "";
            thoughtKey = `live-thought-${Date.now()}`;
            break;

          case "done":
            setIsStreaming(false);
            setNodes([]);
            break;
        }
      } catch {
        // ignore parse errors
      }
    };

    ws.onclose = () => setIsStreaming(false);
    ws.onerror = () => setIsStreaming(false);

    return () => {
      ws.close();
      setIsStreaming(false);
      setNodes([]);
    };
  }, [agentId, processState]);

  return { streamingNodes: nodes, isStreaming };
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const AUTO_REFRESH_INTERVAL = 5_000;

export default function KernelTop() {
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [selectedProcess, setSelectedProcess] = useState<string | null>(null);

  const statsQuery = useQuery({
    queryKey: ["kernel-stats"],
    queryFn: () => api.get<SystemStats>("/api/v1/kernel/stats"),
    refetchInterval: autoRefresh ? AUTO_REFRESH_INTERVAL : false,
  });

  const processesQuery = useQuery({
    queryKey: ["kernel-processes"],
    queryFn: () => api.get<ProcessStats[]>("/api/v1/kernel/processes"),
    refetchInterval: autoRefresh ? AUTO_REFRESH_INTERVAL : false,
  });

  const turnsQuery = useQuery({
    queryKey: ["process-turns", selectedProcess],
    queryFn: () =>
      api.get<TurnTrace[]>(
        `/api/v1/kernel/processes/${selectedProcess}/turns`,
      ),
    enabled: !!selectedProcess,
    refetchInterval: autoRefresh ? AUTO_REFRESH_INTERVAL : false,
  });

  const stats = statsQuery.data;
  const processes = processesQuery.data ?? [];

  const selectedProcessState = processes.find(
    (p) => p.agent_id === selectedProcess
  )?.state ?? null;

  const { streamingNodes, isStreaming } = useProcessStream(
    selectedProcess,
    selectedProcessState
  );

  const handleRefresh = () => {
    statsQuery.refetch();
    processesQuery.refetch();
    if (selectedProcess) turnsQuery.refetch();
  };

  const handleRowClick = (agentId: string) => {
    setSelectedProcess((prev) => (prev === agentId ? null : agentId));
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold">Kernel Top</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Real-time kernel process monitor
          </p>
        </div>
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <Switch
              id="auto-refresh"
              checked={autoRefresh}
              onCheckedChange={setAutoRefresh}
            />
            <Label htmlFor="auto-refresh" className="text-sm text-muted-foreground">
              Auto-refresh
            </Label>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={statsQuery.isFetching || processesQuery.isFetching}
            className="gap-1.5"
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${
                statsQuery.isFetching || processesQuery.isFetching
                  ? "animate-spin"
                  : ""
              }`}
            />
            Refresh
          </Button>
        </div>
      </div>

      {/* Stat Cards */}
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Active Processes
            </CardTitle>
            <Activity className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statsQuery.isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">
                {stats?.active_processes ?? 0}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              {stats
                ? `${stats.global_semaphore_available} slots available`
                : "Loading..."}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Total Spawned
            </CardTitle>
            <Cpu className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statsQuery.isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">
                {stats?.total_spawned ?? 0}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              {stats
                ? `${stats.total_completed} completed, ${stats.total_failed} failed`
                : "Loading..."}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Total Tokens
            </CardTitle>
            <Hash className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statsQuery.isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">
                {formatTokens(stats?.total_tokens_consumed ?? 0)}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              Across all processes
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">Uptime</CardTitle>
            <Clock className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statsQuery.isLoading ? (
              <Skeleton className="h-8 w-24" />
            ) : (
              <div className="text-2xl font-bold">
                {formatUptime(stats?.uptime_ms ?? 0)}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              Kernel runtime
            </p>
          </CardContent>
        </Card>
      </div>

      {/* Process Table */}
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <CardTitle className="flex items-center gap-2 text-base">
              <Zap className="h-4 w-4" />
              Processes
              <Badge variant="secondary" className="ml-1 text-xs">
                {processes.length}
              </Badge>
            </CardTitle>
          </div>
        </CardHeader>
        <CardContent className="pt-0">
          {processesQuery.isLoading ? (
            <div className="space-y-2">
              {Array.from({ length: 3 }).map((_, i) => (
                <Skeleton key={i} className="h-12 w-full" />
              ))}
            </div>
          ) : processes.length === 0 ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              No active processes
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-6" />
                  <TableHead>Agent</TableHead>
                  <TableHead>State</TableHead>
                  <TableHead className="text-right">Uptime</TableHead>
                  <TableHead className="text-right">LLM Calls</TableHead>
                  <TableHead className="text-right">Tool Calls</TableHead>
                  <TableHead className="text-right">Tokens</TableHead>
                  <TableHead className="text-right">Last Activity</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {processes.map((p) => (
                  <Fragment key={p.agent_id}>
                    <TableRow
                      className="cursor-pointer transition-colors hover:bg-muted/50"
                      data-state={selectedProcess === p.agent_id ? "selected" : undefined}
                      onClick={() => handleRowClick(p.agent_id)}
                    >
                      <TableCell className="w-6 px-2">
                        {selectedProcess === p.agent_id ? (
                          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
                        ) : (
                          <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
                        )}
                      </TableCell>
                      <TableCell>
                        <div>
                          <span className="font-medium">
                            {p.manifest_name}
                          </span>
                          {p.parent_id && (
                            <span className="ml-1.5 text-xs text-muted-foreground">
                              (child)
                            </span>
                          )}
                        </div>
                        <span className="font-mono text-xs text-muted-foreground">
                          {p.agent_id.slice(0, 8)}
                        </span>
                      </TableCell>
                      <TableCell>
                        <Badge
                          variant={stateColor(p.state)}
                          className="text-xs"
                        >
                          {p.state}
                        </Badge>
                        {p.finished_at && (
                          <span className="ml-1.5 text-xs text-muted-foreground">
                            {formatRelativeTime(p.finished_at)}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="text-right font-mono text-xs">
                        {formatUptime(p.uptime_ms)}
                      </TableCell>
                      <TableCell className="text-right font-mono text-xs">
                        {p.llm_calls}
                      </TableCell>
                      <TableCell className="text-right font-mono text-xs">
                        {p.tool_calls}
                      </TableCell>
                      <TableCell className="text-right font-mono text-xs">
                        {formatTokens(p.tokens_consumed)}
                      </TableCell>
                      <TableCell className="text-right text-xs text-muted-foreground">
                        {formatRelativeTime(p.last_activity)}
                      </TableCell>
                    </TableRow>
                    {selectedProcess === p.agent_id && (
                      <TableRow>
                        <TableCell colSpan={8} className="bg-muted/20 p-4">
                          <div className="space-y-3">
                            <div className="flex items-center gap-2 text-sm font-medium">
                              <Zap className="h-3.5 w-3.5" />
                              Cascade Viewer
                            </div>
                            {turnsQuery.isLoading ? (
                              <div className="space-y-2">
                                <Skeleton className="h-16 w-full" />
                                <Skeleton className="h-16 w-full" />
                              </div>
                            ) : turnsQuery.isError ? (
                              <div className="text-sm text-muted-foreground italic">
                                Failed to load turn traces
                              </div>
                            ) : (
                              <CascadeViewer
                                traces={turnsQuery.data ?? []}
                                streamingNodes={streamingNodes}
                                isStreaming={isStreaming}
                              />
                            )}
                          </div>
                        </TableCell>
                      </TableRow>
                    )}
                  </Fragment>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
