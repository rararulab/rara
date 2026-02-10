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
import type { Notification, NotificationStatistics } from "@/api/types";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { RefreshCw, Bell, Clock, CheckCircle, XCircle } from "lucide-react";

function formatDate(dateStr: string | null): string {
  if (!dateStr) return "-";
  return new Date(dateStr).toLocaleString();
}

function truncate(text: string, maxLen: number): string {
  if (text.length <= maxLen) return text;
  return text.slice(0, maxLen) + "...";
}

function StatusBadge({ status }: { status: string }) {
  switch (status) {
    case "sent":
      return (
        <Badge className="bg-green-100 text-green-800 hover:bg-green-100">
          sent
        </Badge>
      );
    case "failed":
      return <Badge variant="destructive">failed</Badge>;
    case "pending":
      return <Badge variant="secondary">pending</Badge>;
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

function ChannelBadge({ channel }: { channel: string }) {
  return <Badge variant="outline">{channel}</Badge>;
}

function StatsCards({
  stats,
  isLoading,
}: {
  stats: NotificationStatistics | undefined;
  isLoading: boolean;
}) {
  const cards = [
    {
      title: "Total",
      value: stats?.total,
      icon: Bell,
    },
    {
      title: "Pending",
      value: stats?.pending,
      icon: Clock,
    },
    {
      title: "Sent",
      value: stats?.sent,
      icon: CheckCircle,
    },
    {
      title: "Failed",
      value: stats?.failed,
      icon: XCircle,
    },
  ];

  return (
    <div className="grid gap-4 md:grid-cols-4">
      {cards.map((card) => (
        <Card key={card.title}>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">{card.title}</CardTitle>
            <card.icon className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            {isLoading ? (
              <Skeleton className="h-8 w-16" />
            ) : (
              <div className="text-2xl font-bold">{card.value ?? 0}</div>
            )}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

export default function Notifications() {
  const queryClient = useQueryClient();
  const [channelFilter, setChannelFilter] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<string>("all");

  const buildQueryParams = () => {
    const params = new URLSearchParams();
    if (channelFilter !== "all") params.set("channel", channelFilter);
    if (statusFilter !== "all") params.set("status", statusFilter);
    const qs = params.toString();
    return qs ? `?${qs}` : "";
  };

  const statsQuery = useQuery({
    queryKey: ["notifications", "stats"],
    queryFn: () =>
      api.get<NotificationStatistics>("/api/v1/notifications/stats"),
  });

  const notificationsQuery = useQuery({
    queryKey: ["notifications", channelFilter, statusFilter],
    queryFn: () =>
      api.get<Notification[]>(`/api/v1/notifications${buildQueryParams()}`),
  });

  const retryMutation = useMutation({
    mutationFn: (id: string) =>
      api.post<Notification>(`/api/v1/notifications/${id}/retry`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
    },
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Notifications</h1>
        <p className="text-muted-foreground mt-2">
          View alerts and notification history.
        </p>
      </div>

      <StatsCards stats={statsQuery.data} isLoading={statsQuery.isLoading} />

      <div className="flex items-center gap-4">
        <Select value={channelFilter} onValueChange={setChannelFilter}>
          <SelectTrigger className="w-[160px]">
            <SelectValue placeholder="Channel" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All Channels</SelectItem>
            <SelectItem value="telegram">Telegram</SelectItem>
            <SelectItem value="email">Email</SelectItem>
            <SelectItem value="webhook">Webhook</SelectItem>
          </SelectContent>
        </Select>

        <Select value={statusFilter} onValueChange={setStatusFilter}>
          <SelectTrigger className="w-[160px]">
            <SelectValue placeholder="Status" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All Statuses</SelectItem>
            <SelectItem value="pending">Pending</SelectItem>
            <SelectItem value="sent">Sent</SelectItem>
            <SelectItem value="failed">Failed</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <Card>
        <CardContent className="p-0">
          {notificationsQuery.isLoading ? (
            <div className="space-y-4 p-6">
              {Array.from({ length: 5 }).map((_, i) => (
                <div key={i} className="flex items-center gap-4">
                  <Skeleton className="h-6 w-20" />
                  <Skeleton className="h-6 w-16" />
                  <Skeleton className="h-6 flex-1" />
                  <Skeleton className="h-6 w-32" />
                </div>
              ))}
            </div>
          ) : !notificationsQuery.data?.length ? (
            <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
              <Bell className="h-12 w-12 mb-4 opacity-50" />
              <p className="text-lg font-medium">No notifications</p>
              <p className="text-sm">
                Notifications will appear here when they are created.
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b bg-muted/50">
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">
                      Channel
                    </th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">
                      Status
                    </th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">
                      Body
                    </th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">
                      Created At
                    </th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">
                      Actions
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {notificationsQuery.data.map((notification) => (
                    <tr key={notification.id} className="border-b last:border-0">
                      <td className="px-4 py-3">
                        <ChannelBadge channel={notification.channel} />
                      </td>
                      <td className="px-4 py-3">
                        <StatusBadge status={notification.status} />
                      </td>
                      <td className="px-4 py-3 text-sm max-w-md">
                        {truncate(notification.body, 80)}
                      </td>
                      <td className="px-4 py-3 text-sm text-muted-foreground whitespace-nowrap">
                        {formatDate(notification.created_at)}
                      </td>
                      <td className="px-4 py-3">
                        {notification.status === "failed" && (
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() =>
                              retryMutation.mutate(notification.id)
                            }
                            disabled={retryMutation.isPending}
                          >
                            <RefreshCw className="h-3 w-3 mr-1" />
                            Retry
                          </Button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
