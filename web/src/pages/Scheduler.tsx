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
import type { ScheduledTask, TaskRunRecord } from "@/api/types";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { Clock, History, CalendarClock } from "lucide-react";

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

export default function Scheduler() {
  const tasksQuery = useQuery({
    queryKey: ["scheduler", "tasks"],
    queryFn: () => api.get<ScheduledTask[]>("/api/v1/scheduler/tasks"),
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Scheduler</h1>
        <p className="text-muted-foreground mt-2">
          Configure automated tasks and schedules.
        </p>
      </div>

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
          <CardContent className="flex flex-col items-center justify-center py-12 text-muted-foreground">
            <CalendarClock className="h-12 w-12 mb-4 opacity-50" />
            <p className="text-lg font-medium">No scheduled tasks</p>
            <p className="text-sm">
              Scheduled tasks will appear here when they are configured.
            </p>
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
