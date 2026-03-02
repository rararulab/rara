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

import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { ScheduledTask } from "@/api/types";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Activity, Clock } from "lucide-react";

export default function AgentStatus() {
  const agentJobsQuery = useQuery({
    queryKey: ["scheduler", "tasks"],
    queryFn: () => api.get<ScheduledTask[]>("/api/v1/scheduler/tasks"),
  });

  const agentJobCount = agentJobsQuery.data?.length ?? 0;

  return (
    <div className="space-y-6 p-6">
      <div>
        <h2 className="text-xl font-bold">Agent Status</h2>
        <p className="text-muted-foreground mt-1 text-sm">
          Overview of the agent and its scheduled tasks.
        </p>
      </div>

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        {/* Scheduler Job Count Card */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Scheduled Tasks
            </CardTitle>
            <Clock className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {agentJobsQuery.isLoading ? (
              <Skeleton className="h-8 w-12" />
            ) : (
              <div className="text-2xl font-bold">{agentJobCount}</div>
            )}
            <p className="text-xs text-muted-foreground mt-1">
              Active scheduler tasks
            </p>
          </CardContent>
        </Card>

        {/* Activity Feed Placeholder */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">
              Activity Feed
            </CardTitle>
            <Activity className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <p className="text-sm text-muted-foreground">
              Agent activity feed coming soon.
            </p>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
