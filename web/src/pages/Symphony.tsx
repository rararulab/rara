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

import { useState, useEffect, useRef, useCallback } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  AlertTriangle,
  Bot,
  Database,
  GitBranch,
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

interface RunInfo {
  issue_id: string;
  repo: string;
  title: string;
  workspace_path: string;
  branch: string;
  started_at: string;
}

interface RetryInfo {
  issue_id: string;
  attempt: number;
}

interface ConfigSummary {
  enabled: boolean;
  poll_interval_secs: number;
  max_concurrent_agents: number;
  repos: string[];
}

interface SymphonySnapshot {
  running: RunInfo[];
  claimed: string[];
  retries: RetryInfo[];
  config_summary: ConfigSummary;
  updated_at: string;
}

interface SymphonyEventLog {
  timestamp: string;
  kind: string;
  issue_id: string | null;
  detail: string;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

function eventKindVariant(
  kind: string,
): "default" | "secondary" | "destructive" | "outline" {
  const k = kind.toLowerCase();
  if (k.includes("error") || k.includes("fail")) return "destructive";
  if (k.includes("start") || k.includes("spawn")) return "default";
  if (k.includes("complete") || k.includes("done") || k.includes("success"))
    return "secondary";
  return "outline";
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const AUTO_REFRESH_INTERVAL = 5_000;
const MAX_EVENTS = 100;

export default function Symphony() {
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [events, setEvents] = useState<SymphonyEventLog[]>([]);
  const [userScrolled, setUserScrolled] = useState(false);
  const eventLogRef = useRef<HTMLDivElement>(null);

  // ---- Status polling ----
  const statusQuery = useQuery({
    queryKey: ["symphony-status"],
    queryFn: () => api.get<SymphonySnapshot>("/api/symphony/status"),
    refetchInterval: autoRefresh ? AUTO_REFRESH_INTERVAL : false,
  });

  const snapshot = statusQuery.data;

  const handleRefresh = () => {
    statusQuery.refetch();
  };

  // ---- SSE event stream ----
  useEffect(() => {
    if (!autoRefresh) return;
    const controller = new AbortController();
    const baseUrl = import.meta.env.VITE_API_URL || "";
    const token = localStorage.getItem("access_token") ?? "";

    (async () => {
      try {
        const res = await fetch(`${baseUrl}/api/symphony/events`, {
          headers: token ? { Authorization: `Bearer ${token}` } : {},
          signal: controller.signal,
        });
        if (!res.ok || !res.body) return;
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";

        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop() ?? "";

          for (const line of lines) {
            if (line.startsWith("data:")) {
              const data = line.slice(5).trim();
              if (data) {
                try {
                  const log = JSON.parse(data) as SymphonyEventLog;
                  setEvents((prev) => [log, ...prev].slice(0, MAX_EVENTS));
                } catch {
                  /* skip malformed */
                }
              }
            }
          }
        }
      } catch {
        // aborted or network error
      }
    })();

    return () => controller.abort();
  }, [autoRefresh]);

  // ---- Auto-scroll for event log ----
  const handleEventLogScroll = useCallback(() => {
    const el = eventLogRef.current;
    if (!el) return;
    // If user scrolled up (not at top), disable auto-scroll
    setUserScrolled(el.scrollTop > 0);
  }, []);

  useEffect(() => {
    if (!userScrolled && eventLogRef.current) {
      eventLogRef.current.scrollTop = 0;
    }
  }, [events, userScrolled]);

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold">Symphony</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Autonomous coding agent orchestrator
          </p>
        </div>
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <Switch
              id="auto-refresh"
              checked={autoRefresh}
              onCheckedChange={setAutoRefresh}
            />
            <Label
              htmlFor="auto-refresh"
              className="text-sm text-muted-foreground"
            >
              Auto-refresh
            </Label>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={statusQuery.isFetching}
            className="gap-1.5"
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${
                statusQuery.isFetching ? "animate-spin" : ""
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
              Running Agents
            </CardTitle>
            <Bot className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statusQuery.isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">
                {snapshot?.running.length ?? 0}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              {snapshot
                ? `${snapshot.config_summary.max_concurrent_agents} max slots`
                : "Loading..."}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Claimed Issues
            </CardTitle>
            <GitBranch className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statusQuery.isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">
                {snapshot?.claimed.length ?? 0}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              Issues in progress
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Pending Retries
            </CardTitle>
            <AlertTriangle className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statusQuery.isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">
                {snapshot?.retries.length ?? 0}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              Awaiting retry
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Tracked Repos
            </CardTitle>
            <Database className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {statusQuery.isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">
                {snapshot?.config_summary.repos.length ?? 0}
              </div>
            )}
            <p className="mt-1 text-xs text-muted-foreground truncate">
              {snapshot
                ? snapshot.config_summary.repos.join(", ") || "None"
                : "Loading..."}
            </p>
          </CardContent>
        </Card>
      </div>

      {/* Running Agents Table */}
      {snapshot && snapshot.running.length > 0 && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <Zap className="h-4 w-4" />
              Running Agents
              <Badge variant="secondary" className="ml-1 text-xs">
                {snapshot.running.length}
              </Badge>
            </CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Issue</TableHead>
                  <TableHead>Repo</TableHead>
                  <TableHead>Title</TableHead>
                  <TableHead>Branch</TableHead>
                  <TableHead>Workspace</TableHead>
                  <TableHead className="text-right">Started</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {snapshot.running.map((run) => (
                  <TableRow key={run.issue_id}>
                    <TableCell className="font-mono text-xs">
                      {run.issue_id}
                    </TableCell>
                    <TableCell className="text-xs">{run.repo}</TableCell>
                    <TableCell className="text-sm">{run.title}</TableCell>
                    <TableCell className="font-mono text-xs">
                      {run.branch}
                    </TableCell>
                    <TableCell
                      className="max-w-[200px] truncate font-mono text-xs text-muted-foreground"
                      title={run.workspace_path}
                    >
                      {run.workspace_path}
                    </TableCell>
                    <TableCell className="text-right text-xs text-muted-foreground">
                      {formatRelativeTime(run.started_at)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {/* Retries Table */}
      {snapshot && snapshot.retries.length > 0 && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <AlertTriangle className="h-4 w-4" />
              Pending Retries
              <Badge variant="destructive" className="ml-1 text-xs">
                {snapshot.retries.length}
              </Badge>
            </CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Issue</TableHead>
                  <TableHead className="text-right">Attempt</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {snapshot.retries.map((r) => (
                  <TableRow key={r.issue_id}>
                    <TableCell className="font-mono text-xs">
                      {r.issue_id}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs">
                      {r.attempt}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {/* Event Log */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="flex items-center gap-2 text-base">
            <Activity className="h-4 w-4" />
            Event Log
            <Badge variant="secondary" className="ml-1 text-xs">
              {events.length}
            </Badge>
            {userScrolled && (
              <Button
                variant="ghost"
                size="sm"
                className="ml-auto h-6 text-xs"
                onClick={() => {
                  setUserScrolled(false);
                  if (eventLogRef.current) eventLogRef.current.scrollTop = 0;
                }}
              >
                Scroll to top
              </Button>
            )}
          </CardTitle>
        </CardHeader>
        <CardContent className="pt-0">
          <div
            ref={eventLogRef}
            className="max-h-[400px] overflow-y-auto space-y-1"
            onScroll={handleEventLogScroll}
          >
            {events.length === 0 ? (
              <div className="py-8 text-center text-sm text-muted-foreground">
                {autoRefresh
                  ? "Waiting for events..."
                  : "Enable auto-refresh to receive events"}
              </div>
            ) : (
              events.map((ev, i) => (
                <div
                  key={`${ev.timestamp}-${i}`}
                  className="flex items-start gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted/50"
                >
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {formatRelativeTime(ev.timestamp)}
                  </span>
                  <Badge
                    variant={eventKindVariant(ev.kind)}
                    className="shrink-0 text-xs"
                  >
                    {ev.kind}
                  </Badge>
                  {ev.issue_id && (
                    <span className="shrink-0 font-mono text-xs text-muted-foreground">
                      {ev.issue_id}
                    </span>
                  )}
                  <span className="text-xs">{ev.detail}</span>
                </div>
              ))
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
