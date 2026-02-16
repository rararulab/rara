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
import type { AgentJob, AgentTrigger, ScheduledTask, TaskRunRecord } from "@/api/types";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { Bot, Clock, History, CalendarClock, Timer, Repeat } from "lucide-react";

function formatDate(dateStr: string | null): string {
  if (!dateStr) return "-";
  return new Date(dateStr).toLocaleString();
}

function RunStatusBadge({ status }: { status: string }) {
  switch (status) {
    case "success":
      return (
        <Badge className="bg-green-100 text-green-800 hover:bg-green-100">
          success
        </Badge>
      );
    case "failed":
      return <Badge variant="destructive">failed</Badge>;
    case "running":
      return (
        <Badge className="bg-blue-100 text-blue-800 hover:bg-blue-100">
          running
        </Badge>
      );
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

function TaskHistory({ taskId }: { taskId: string }) {
  const historyQuery = useQuery({
    queryKey: ["scheduler", "history", taskId],
    queryFn: () =>
      api.get<TaskRunRecord[]>(
        `/api/v1/scheduler/tasks/${taskId}/history?limit=10`
      ),
  });

  if (historyQuery.isLoading) {
    return (
      <div className="space-y-2 p-4">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={i} className="h-8 w-full" />
        ))}
      </div>
    );
  }

  if (!historyQuery.data?.length) {
    return (
      <div className="py-6 text-center text-sm text-muted-foreground">
        No run history available.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full">
        <thead>
          <tr className="border-b bg-muted/50">
            <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
              Status
            </th>
            <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
              Started At
            </th>
            <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
              Finished At
            </th>
            <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
              Error
            </th>
          </tr>
        </thead>
        <tbody>
          {historyQuery.data.map((run) => (
            <tr key={run.id} className="border-b last:border-0">
              <td className="px-4 py-2">
                <RunStatusBadge status={run.status} />
              </td>
              <td className="px-4 py-2 text-sm text-muted-foreground whitespace-nowrap">
                {formatDate(run.started_at)}
              </td>
              <td className="px-4 py-2 text-sm text-muted-foreground whitespace-nowrap">
                {formatDate(run.finished_at)}
              </td>
              <td className="px-4 py-2 text-sm text-destructive">
                {run.error_message || "-"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function TaskCard({ task }: { task: ScheduledTask }) {
  const queryClient = useQueryClient();
  const [showHistory, setShowHistory] = useState(false);

  const enableMutation = useMutation({
    mutationFn: () =>
      api.post<ScheduledTask>(`/api/v1/scheduler/tasks/${task.id}/enable`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["scheduler", "tasks"] });
    },
  });

  const disableMutation = useMutation({
    mutationFn: () =>
      api.post<ScheduledTask>(`/api/v1/scheduler/tasks/${task.id}/disable`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["scheduler", "tasks"] });
    },
  });

  const toggleEnabled = () => {
    if (task.enabled) {
      disableMutation.mutate();
    } else {
      enableMutation.mutate();
    }
  };

  const isToggling = enableMutation.isPending || disableMutation.isPending;

  return (
    <Card>
      <CardContent className="p-6">
        <div className="flex items-start justify-between">
          <div className="space-y-1">
            <h3 className="font-semibold text-lg">{task.name}</h3>
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Clock className="h-3.5 w-3.5" />
              <code className="bg-muted px-1.5 py-0.5 rounded text-xs">
                {task.cron_expression}
              </code>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">
              {task.enabled ? "Enabled" : "Disabled"}
            </span>
            <Switch
              checked={task.enabled}
              onCheckedChange={toggleEnabled}
              disabled={isToggling}
            />
          </div>
        </div>

        <Separator className="my-4" />

        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <span className="text-muted-foreground">Last run:</span>{" "}
            <span className="font-medium">
              {formatDate(task.last_run_at)}
            </span>
          </div>
          <div>
            <span className="text-muted-foreground">Next run:</span>{" "}
            <span className="font-medium">
              {formatDate(task.next_run_at)}
            </span>
          </div>
        </div>

        <div className="mt-4">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setShowHistory(!showHistory)}
          >
            <History className="h-3 w-3 mr-1" />
            {showHistory ? "Hide History" : "View History"}
          </Button>
        </div>

        {showHistory && (
          <div className="mt-4 border rounded-md">
            <TaskHistory taskId={task.id} />
          </div>
        )}
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Agent Jobs (created by AI agent)
// ---------------------------------------------------------------------------

function formatTrigger(trigger: AgentTrigger): { icon: React.ReactNode; label: string } {
  switch (trigger.type) {
    case "cron":
      return { icon: <Clock className="h-3.5 w-3.5" />, label: trigger.expr };
    case "delay":
      return { icon: <Timer className="h-3.5 w-3.5" />, label: `once at ${new Date(trigger.run_at).toLocaleString()}` };
    case "interval":
      return {
        icon: <Repeat className="h-3.5 w-3.5" />,
        label: trigger.seconds >= 3600
          ? `every ${(trigger.seconds / 3600).toFixed(1)}h`
          : trigger.seconds >= 60
            ? `every ${Math.round(trigger.seconds / 60)}m`
            : `every ${trigger.seconds}s`,
      };
  }
}

function AgentJobCard({ job }: { job: AgentJob }) {
  const trigger = formatTrigger(job.trigger);

  return (
    <Card>
      <CardContent className="p-5">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0 flex-1 space-y-2">
            <p className="text-sm leading-relaxed">{job.message}</p>
            <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
              <div className="flex items-center gap-1">
                {trigger.icon}
                <code className="bg-muted px-1.5 py-0.5 rounded">{trigger.label}</code>
              </div>
              <span>Created {formatDate(job.created_at)}</span>
              {job.last_run_at && <span>Last run {formatDate(job.last_run_at)}</span>}
            </div>
          </div>
          <Badge variant={job.enabled ? "default" : "outline"} className="shrink-0">
            {job.enabled ? "Active" : "Disabled"}
          </Badge>
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Exported panels for use in tab containers
// ---------------------------------------------------------------------------

/** Domain scheduled tasks panel (cron tasks with history). */
export function DomainSchedulerPanel() {
  const tasksQuery = useQuery({
    queryKey: ["scheduler", "tasks"],
    queryFn: () => api.get<ScheduledTask[]>("/api/v1/scheduler/tasks"),
  });

  return (
    <div className="space-y-4">
      {tasksQuery.isLoading ? (
        <div className="space-y-4">
          {Array.from({ length: 3 }).map((_, i) => (
            <Card key={i}>
              <CardContent className="p-6 space-y-4">
                <div className="flex justify-between">
                  <Skeleton className="h-6 w-48" />
                  <Skeleton className="h-5 w-12" />
                </div>
                <Skeleton className="h-4 w-32" />
                <Skeleton className="h-px w-full" />
                <div className="flex gap-8">
                  <Skeleton className="h-4 w-40" />
                  <Skeleton className="h-4 w-40" />
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : !tasksQuery.data?.length ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-10 text-muted-foreground">
            <CalendarClock className="h-10 w-10 mb-3 opacity-30" />
            <p className="text-sm">No scheduled tasks configured.</p>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-4">
          {tasksQuery.data.map((task) => (
            <TaskCard key={task.id} task={task} />
          ))}
        </div>
      )}
    </div>
  );
}

/** Agent jobs panel (AI agent-created scheduled jobs). */
export function AgentJobsPanel() {
  const agentJobsQuery = useQuery({
    queryKey: ["agent-scheduler", "jobs"],
    queryFn: () => api.get<AgentJob[]>("/api/v1/agent-scheduler/jobs"),
    refetchInterval: 30_000,
  });

  return (
    <div className="space-y-4">
      {agentJobsQuery.isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 2 }).map((_, i) => (
            <Card key={i}>
              <CardContent className="p-5 space-y-3">
                <Skeleton className="h-4 w-3/4" />
                <Skeleton className="h-3 w-48" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : !agentJobsQuery.data?.length ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-10 text-muted-foreground">
            <Bot className="h-10 w-10 mb-3 opacity-30" />
            <p className="text-sm">No agent jobs yet. The agent can create scheduled tasks via tools.</p>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-3">
          {agentJobsQuery.data.map((job) => (
            <AgentJobCard key={job.id} job={job} />
          ))}
        </div>
      )}
    </div>
  );
}

export default function Scheduler() {
  return (
    <div className="space-y-8">
      <div className="space-y-4">
        <div>
          <div className="flex items-center gap-2">
            <Bot className="h-5 w-5 text-muted-foreground" />
            <h1 className="text-2xl font-bold">Agent Jobs</h1>
          </div>
          <p className="text-muted-foreground mt-1">
            Scheduled jobs created by the AI agent.
          </p>
        </div>
        <AgentJobsPanel />
      </div>

      <Separator />

      <div className="space-y-4">
        <div>
          <div className="flex items-center gap-2">
            <CalendarClock className="h-5 w-5 text-muted-foreground" />
            <h2 className="text-xl font-bold">Scheduled Tasks</h2>
          </div>
          <p className="text-muted-foreground mt-1">
            System cron tasks and their run history.
          </p>
        </div>
        <DomainSchedulerPanel />
      </div>
    </div>
  );
}
