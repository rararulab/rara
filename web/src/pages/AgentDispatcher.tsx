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

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type {
  DispatcherStatus,
  TaskRecord,
  AgentTaskKind,
  AgentTaskKindValue,
  TaskPriority,
  TaskStatus,
} from "@/api/types";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Activity,
  CheckCircle2,
  AlertTriangle,
  Copy,
  Clock,
  ListOrdered,
  History,
  XCircle,
  Loader2,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}m ${seconds}s`;
  const hours = Math.floor(minutes / 60);
  const remainMinutes = minutes % 60;
  return `${hours}h ${remainMinutes}m`;
}

function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  const minutes = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  if (minutes < 60) return `${minutes}m ${secs}s`;
  const hours = Math.floor(minutes / 60);
  const remainMinutes = minutes % 60;
  return `${hours}h ${remainMinutes}m`;
}

function normalizeTaskKind(kind: AgentTaskKindValue): AgentTaskKind {
  if (typeof kind === "string") return kind;
  return kind.type;
}

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  const hours = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  if (hours < 24) return `${hours}h ${mins}m`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return `${days}d ${remHours}h`;
}

function formatRelativeTime(dateStr: string): string {
  if (!dateStr) return "-";
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  if (Number.isNaN(then)) return "-";
  const diffMs = now - then;
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return "just now";
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin} min ago`;
  const diffHour = Math.floor(diffMin / 60);
  if (diffHour < 24) return `${diffHour}h ago`;
  const diffDay = Math.floor(diffHour / 24);
  return `${diffDay}d ago`;
}

function PriorityBadge({ priority }: { priority: TaskPriority }) {
  switch (priority) {
    case "urgent":
      return (
        <Badge className="bg-red-100 text-red-800 hover:bg-red-100">
          urgent
        </Badge>
      );
    case "high":
      return (
        <Badge className="bg-orange-100 text-orange-800 hover:bg-orange-100">
          high
        </Badge>
      );
    case "normal":
      return (
        <Badge className="bg-blue-100 text-blue-800 hover:bg-blue-100">
          normal
        </Badge>
      );
    case "low":
      return <Badge variant="outline">low</Badge>;
    default:
      return <Badge variant="outline">{priority}</Badge>;
  }
}

function TaskStatusBadge({ status }: { status: TaskStatus }) {
  switch (status) {
    case "completed":
      return (
        <Badge className="bg-green-100 text-green-800 hover:bg-green-100">
          completed
        </Badge>
      );
    case "error":
      return <Badge variant="destructive">error</Badge>;
    case "cancelled":
      return (
        <Badge className="bg-yellow-100 text-yellow-800 hover:bg-yellow-100">
          cancelled
        </Badge>
      );
    case "deduped":
      return (
        <Badge className="bg-gray-100 text-gray-600 hover:bg-gray-100">
          deduped
        </Badge>
      );
    case "running":
      return (
        <Badge className="bg-blue-100 text-blue-800 hover:bg-blue-100">
          running
        </Badge>
      );
    case "queued":
      return <Badge variant="outline">queued</Badge>;
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

function KindBadge({ kind }: { kind: AgentTaskKindValue }) {
  const label = normalizeTaskKind(kind);
  switch (label) {
    case "proactive":
      return <Badge variant="secondary">proactive</Badge>;
    case "scheduled":
      return (
        <Badge className="bg-purple-100 text-purple-800 hover:bg-purple-100">
          scheduled
        </Badge>
      );
    case "pipeline":
      return (
        <Badge className="bg-indigo-100 text-indigo-800 hover:bg-indigo-100">
          pipeline
        </Badge>
      );
    default:
      return <Badge variant="outline">{label}</Badge>;
  }
}

// ---------------------------------------------------------------------------
// Stat Card
// ---------------------------------------------------------------------------

interface StatCardProps {
  title: string;
  value: string;
  icon: React.ReactNode;
  description?: string;
}

function StatCard({ title, value, icon, description }: StatCardProps) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">
          {title}
        </CardTitle>
        <span className="text-muted-foreground">{icon}</span>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{value}</div>
        {description && (
          <CardDescription className="mt-1">{description}</CardDescription>
        )}
      </CardContent>
    </Card>
  );
}

function StatCardSkeleton() {
  return (
    <Card>
      <CardHeader className="pb-2">
        <Skeleton className="h-4 w-24" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-8 w-16 mb-1" />
        <Skeleton className="h-3 w-32" />
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Running Tasks Section
// ---------------------------------------------------------------------------

function RunningTasksSection({
  status,
  onCancel,
  isCancelling,
}: {
  status: DispatcherStatus;
  onCancel: (taskId: string) => void;
  isCancelling: boolean;
}) {
  if (!status.running.length) {
    return (
      <Card>
        <CardContent className="flex flex-col items-center justify-center py-10 text-muted-foreground">
          <Activity className="h-10 w-10 mb-3 opacity-30" />
          <p className="text-sm">No tasks running</p>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardContent className="p-0">
        <div className="overflow-x-auto">
          <table className="w-full">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Kind
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Session
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Priority
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Message
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Elapsed
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {status.running.map((task) => (
                <tr key={task.id} className="border-b last:border-0">
                  <td className="px-4 py-2">
                    <KindBadge kind={task.kind} />
                  </td>
                  <td className="px-4 py-2 text-sm">
                    <code className="bg-muted px-1.5 py-0.5 rounded text-xs">
                      {task.session_key}
                    </code>
                  </td>
                  <td className="px-4 py-2">
                    <PriorityBadge priority={task.priority} />
                  </td>
                  <td className="px-4 py-2 text-sm text-muted-foreground max-w-xs truncate">
                    {task.message_preview ?? "-"}
                  </td>
                  <td className="px-4 py-2 text-sm text-muted-foreground whitespace-nowrap">
                    {typeof task.elapsed_seconds === "number"
                      ? formatElapsed(task.elapsed_seconds)
                      : "-"}
                  </td>
                  <td className="px-4 py-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => onCancel(task.id)}
                      disabled={isCancelling}
                    >
                      <XCircle className="h-3 w-3 mr-1" />
                      Cancel
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Queue Section
// ---------------------------------------------------------------------------

function QueueSection({ status }: { status: DispatcherStatus }) {
  if (!status.queued.length) {
    return (
      <Card>
        <CardContent className="flex flex-col items-center justify-center py-10 text-muted-foreground">
          <ListOrdered className="h-10 w-10 mb-3 opacity-30" />
          <p className="text-sm">Queue empty</p>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardContent className="p-0">
        <div className="overflow-x-auto">
          <table className="w-full">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Kind
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Session
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Priority
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Message
                </th>
                <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                  Waiting
                </th>
              </tr>
            </thead>
            <tbody>
              {status.queued.map((task) => (
                <tr key={task.id} className="border-b last:border-0">
                  <td className="px-4 py-2">
                    <KindBadge kind={task.kind} />
                  </td>
                  <td className="px-4 py-2 text-sm">
                    <code className="bg-muted px-1.5 py-0.5 rounded text-xs">
                      {task.session_key}
                    </code>
                  </td>
                  <td className="px-4 py-2">
                    <PriorityBadge priority={task.priority} />
                  </td>
                  <td className="px-4 py-2 text-sm text-muted-foreground max-w-xs truncate">
                    {task.message_preview ?? "-"}
                  </td>
                  <td className="px-4 py-2 text-sm text-muted-foreground whitespace-nowrap">
                    {formatRelativeTime(task.submitted_at ?? task.created_at ?? "")}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// History Section
// ---------------------------------------------------------------------------

function HistorySection() {
  const [kindFilter, setKindFilter] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [limit, setLimit] = useState(50);

  const historyQuery = useQuery({
    queryKey: ["dispatcher", "history", kindFilter, statusFilter, limit],
    queryFn: () =>
      api.fetchDispatcherHistory({
        limit,
        kind: kindFilter === "all" ? undefined : (kindFilter as AgentTaskKind),
        status:
          statusFilter === "all" ? undefined : (statusFilter as TaskStatus),
      }),
    refetchInterval: 30_000,
  });

  return (
    <div className="space-y-4">
      {/* Filter bar */}
      <div className="flex flex-wrap items-center gap-3">
        <Select value={kindFilter} onValueChange={setKindFilter}>
          <SelectTrigger className="w-[160px]">
            <SelectValue placeholder="Kind" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All Kinds</SelectItem>
            <SelectItem value="proactive">Proactive</SelectItem>
            <SelectItem value="scheduled">Scheduled</SelectItem>
            <SelectItem value="pipeline">Pipeline</SelectItem>
          </SelectContent>
        </Select>

        <Select value={statusFilter} onValueChange={setStatusFilter}>
          <SelectTrigger className="w-[160px]">
            <SelectValue placeholder="Status" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All Statuses</SelectItem>
            <SelectItem value="completed">Completed</SelectItem>
            <SelectItem value="error">Error</SelectItem>
            <SelectItem value="cancelled">Cancelled</SelectItem>
            <SelectItem value="deduped">Deduped</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* Table */}
      {historyQuery.isLoading ? (
        <Card>
          <CardContent className="p-6 space-y-3">
            {Array.from({ length: 5 }).map((_, i) => (
              <Skeleton key={i} className="h-8 w-full" />
            ))}
          </CardContent>
        </Card>
      ) : !historyQuery.data?.length ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-10 text-muted-foreground">
            <History className="h-10 w-10 mb-3 opacity-30" />
            <p className="text-sm">No history records found.</p>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="p-0">
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b bg-muted/50">
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Kind
                    </th>
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Session
                    </th>
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Priority
                    </th>
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Status
                    </th>
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Duration
                    </th>
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Iterations
                    </th>
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Tool Calls
                    </th>
                    <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
                      Time
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {historyQuery.data.map((record: TaskRecord) => (
                    <tr key={record.id} className="border-b last:border-0">
                      <td className="px-4 py-2">
                        <KindBadge kind={record.kind} />
                      </td>
                      <td className="px-4 py-2 text-sm">
                        <code className="bg-muted px-1.5 py-0.5 rounded text-xs">
                          {record.session_key}
                        </code>
                      </td>
                      <td className="px-4 py-2">
                        <PriorityBadge priority={record.priority} />
                      </td>
                      <td className="px-4 py-2">
                        <TaskStatusBadge status={record.status} />
                      </td>
                      <td className="px-4 py-2 text-sm text-muted-foreground whitespace-nowrap">
                        {record.duration_ms != null
                          ? formatDuration(record.duration_ms)
                          : "-"}
                      </td>
                      <td className="px-4 py-2 text-sm text-muted-foreground">
                        {record.iterations ?? "-"}
                      </td>
                      <td className="px-4 py-2 text-sm text-muted-foreground">
                        {record.tool_calls ?? "-"}
                      </td>
                      <td className="px-4 py-2 text-sm text-muted-foreground whitespace-nowrap">
                        {formatRelativeTime(record.submitted_at)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Load more */}
      {historyQuery.data && historyQuery.data.length >= limit && (
        <div className="flex justify-center">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setLimit((prev) => prev + 50)}
          >
            Load more
          </Button>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Page
// ---------------------------------------------------------------------------

export default function AgentDispatcher() {
  const queryClient = useQueryClient();

  const statusQuery = useQuery({
    queryKey: ["dispatcher", "status"],
    queryFn: () => api.fetchDispatcherStatus(),
    refetchInterval: 5_000,
  });

  const cancelMutation = useMutation({
    mutationFn: (taskId: string) => api.cancelDispatcherTask(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["dispatcher", "status"] });
      queryClient.invalidateQueries({ queryKey: ["dispatcher", "history"] });
    },
  });

  const stats = statusQuery.data?.stats;

  return (
    <div className="space-y-8">
      {/* Header */}
      <div>
        <div className="flex items-center gap-2">
          <Activity className="h-5 w-5 text-muted-foreground" />
          <h1 className="text-2xl font-bold">Agent Dispatcher</h1>
          {stats && (
            <span className="text-sm text-muted-foreground ml-2">
              Uptime: {formatUptime(stats.uptime_seconds)}
            </span>
          )}
          {statusQuery.isFetching && !statusQuery.isLoading && (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground ml-2" />
          )}
        </div>
        <p className="text-muted-foreground mt-1">
          Centralized agent task queue with priority scheduling and session-parallel execution.
        </p>
      </div>

      {/* Stats Cards */}
      <div className="grid gap-4 grid-cols-1 sm:grid-cols-2 lg:grid-cols-4">
        {statusQuery.isLoading ? (
          <>
            <StatCardSkeleton />
            <StatCardSkeleton />
            <StatCardSkeleton />
            <StatCardSkeleton />
          </>
        ) : stats ? (
          <>
            <StatCard
              title="Total Submitted"
              value={String(stats.total_submitted)}
              icon={<ListOrdered className="h-4 w-4" />}
              description={`${statusQuery.data!.running.length} running, ${statusQuery.data!.queued.length} queued`}
            />
            <StatCard
              title="Total Completed"
              value={String(stats.total_completed)}
              icon={<CheckCircle2 className="h-4 w-4 text-green-600" />}
            />
            <StatCard
              title="Total Errors"
              value={String(stats.total_errors)}
              icon={<AlertTriangle className="h-4 w-4 text-red-600" />}
            />
            <StatCard
              title="Total Deduped"
              value={String(stats.total_deduped)}
              icon={<Copy className="h-4 w-4" />}
              description={`${stats.total_cancelled} cancelled`}
            />
          </>
        ) : (
          <>
            <StatCard
              title="Total Submitted"
              value="0"
              icon={<ListOrdered className="h-4 w-4" />}
              description="No data yet"
            />
            <StatCard
              title="Total Completed"
              value="0"
              icon={<CheckCircle2 className="h-4 w-4" />}
              description="No data yet"
            />
            <StatCard
              title="Total Errors"
              value="0"
              icon={<AlertTriangle className="h-4 w-4" />}
              description="No data yet"
            />
            <StatCard
              title="Total Deduped"
              value="0"
              icon={<Copy className="h-4 w-4" />}
              description="No data yet"
            />
          </>
        )}
      </div>

      <Separator />

      {/* Running Tasks */}
      <div className="space-y-4">
        <div className="flex items-center gap-2">
          <Activity className="h-5 w-5 text-muted-foreground" />
          <h2 className="text-xl font-bold">Running Tasks</h2>
          {statusQuery.data && (
            <Badge variant="secondary" className="ml-1">
              {statusQuery.data.running.length}
            </Badge>
          )}
        </div>
        {statusQuery.isLoading ? (
          <Card>
            <CardContent className="p-6 space-y-3">
              {Array.from({ length: 2 }).map((_, i) => (
                <Skeleton key={i} className="h-10 w-full" />
              ))}
            </CardContent>
          </Card>
        ) : statusQuery.data ? (
          <RunningTasksSection
            status={statusQuery.data}
            onCancel={(taskId) => cancelMutation.mutate(taskId)}
            isCancelling={cancelMutation.isPending}
          />
        ) : null}
      </div>

      <Separator />

      {/* Queue */}
      <div className="space-y-4">
        <div className="flex items-center gap-2">
          <Clock className="h-5 w-5 text-muted-foreground" />
          <h2 className="text-xl font-bold">Queue</h2>
          {statusQuery.data && (
            <Badge variant="secondary" className="ml-1">
              {statusQuery.data.queued.length}
            </Badge>
          )}
        </div>
        {statusQuery.isLoading ? (
          <Card>
            <CardContent className="p-6 space-y-3">
              {Array.from({ length: 2 }).map((_, i) => (
                <Skeleton key={i} className="h-10 w-full" />
              ))}
            </CardContent>
          </Card>
        ) : statusQuery.data ? (
          <QueueSection status={statusQuery.data} />
        ) : null}
      </div>

      <Separator />

      {/* History */}
      <div className="space-y-4">
        <div className="flex items-center gap-2">
          <History className="h-5 w-5 text-muted-foreground" />
          <h2 className="text-xl font-bold">History</h2>
        </div>
        <HistorySection />
      </div>
    </div>
  );
}
