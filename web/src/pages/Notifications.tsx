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

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type {
  NotificationQueueMessage,
  NotificationQueueMessagesResponse,
  NotificationQueueOverview,
  QueueMessageState,
} from "@/api/types";
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
import { Bell, Clock3, Archive, Hourglass, ListOrdered } from "lucide-react";

const PAGE_SIZE = 50;

function formatDate(value: string | null | undefined): string {
  if (!value) return "-";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString();
}

function truncate(text: string | null | undefined, maxLen = 96): string {
  if (!text) return "-";
  if (text.length <= maxLen) return text;
  return `${text.slice(0, maxLen)}...`;
}

function QueueStateBadge({ state }: { state: QueueMessageState }) {
  if (state === "ready") {
    return <Badge variant="secondary">ready</Badge>;
  }
  if (state === "inflight") {
    return <Badge className="bg-amber-100 text-amber-800 hover:bg-amber-100">inflight</Badge>;
  }
  return <Badge variant="outline">archived</Badge>;
}

function StatsCards({
  overview,
  isLoading,
}: {
  overview: NotificationQueueOverview | undefined;
  isLoading: boolean;
}) {
  const total = (overview?.ready_count ?? 0) + (overview?.inflight_count ?? 0) + (overview?.archived_count ?? 0);
  const cards = [
    { title: "Total", value: total, icon: ListOrdered },
    { title: "Ready", value: overview?.ready_count ?? 0, icon: Clock3 },
    { title: "In Flight", value: overview?.inflight_count ?? 0, icon: Hourglass },
    { title: "Archived", value: overview?.archived_count ?? 0, icon: Archive },
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
            {isLoading ? <Skeleton className="h-8 w-16" /> : <div className="text-2xl font-bold">{card.value}</div>}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

function messageKey(item: NotificationQueueMessage): string {
  const payloadId = typeof item.payload.id === "string" ? item.payload.id : "no-id";
  return `${item.msg_id}-${payloadId}`;
}

export default function Notifications() {
  const [stateFilter, setStateFilter] = useState<QueueMessageState>("ready");
  const [page, setPage] = useState(0);

  const offset = page * PAGE_SIZE;

  const overviewQuery = useQuery({
    queryKey: ["notifications", "queue", "overview"],
    queryFn: () => api.get<NotificationQueueOverview>("/api/v1/notifications/queues/telegram/overview"),
    refetchInterval: 5000,
  });

  const messagesQuery = useQuery({
    queryKey: ["notifications", "queue", "messages", stateFilter, PAGE_SIZE, offset],
    queryFn: () =>
      api.get<NotificationQueueMessagesResponse>(
        `/api/v1/notifications/queues/telegram/messages?state=${stateFilter}&limit=${PAGE_SIZE}&offset=${offset}`,
      ),
    refetchInterval: 5000,
  });

  const stateTotal = useMemo(() => {
    const overview = overviewQuery.data;
    if (!overview) return 0;
    if (stateFilter === "ready") return overview.ready_count;
    if (stateFilter === "inflight") return overview.inflight_count;
    return overview.archived_count;
  }, [overviewQuery.data, stateFilter]);

  const maxPage = Math.max(0, Math.ceil(stateTotal / PAGE_SIZE) - 1);
  const canPrev = page > 0;
  const canNext = page < maxPage;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Notifications</h1>
        <p className="text-muted-foreground mt-2">
          Queue-level observability for telegram delivery (`notification_telegram_dispatch`).
        </p>
      </div>

      <StatsCards overview={overviewQuery.data} isLoading={overviewQuery.isLoading} />

      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-4">
          <Select
            value={stateFilter}
            onValueChange={(next) => {
              setStateFilter(next as QueueMessageState);
              setPage(0);
            }}
          >
            <SelectTrigger className="w-[180px]">
              <SelectValue placeholder="Queue state" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="ready">Ready</SelectItem>
              <SelectItem value="inflight">In Flight</SelectItem>
              <SelectItem value="archived">Archived</SelectItem>
            </SelectContent>
          </Select>

          <p className="text-sm text-muted-foreground">Total in state: {stateTotal}</p>
        </div>

        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" disabled={!canPrev} onClick={() => setPage((p) => Math.max(0, p - 1))}>
            Prev
          </Button>
          <p className="text-sm text-muted-foreground">Page {page + 1}</p>
          <Button variant="outline" size="sm" disabled={!canNext} onClick={() => setPage((p) => p + 1)}>
            Next
          </Button>
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          {messagesQuery.isLoading ? (
            <div className="space-y-4 p-6">
              {Array.from({ length: 5 }).map((_, i) => (
                <div key={i} className="flex items-center gap-4">
                  <Skeleton className="h-6 w-16" />
                  <Skeleton className="h-6 w-20" />
                  <Skeleton className="h-6 w-28" />
                  <Skeleton className="h-6 flex-1" />
                </div>
              ))}
            </div>
          ) : !messagesQuery.data?.items.length ? (
            <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
              <Bell className="h-12 w-12 mb-4 opacity-50" />
              <p className="text-lg font-medium">No messages in this state</p>
              <p className="text-sm">Try another state filter or wait for new queue activity.</p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b bg-muted/50">
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">Msg ID</th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">State</th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">Read Count</th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">Enqueued At</th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">VT / Archived</th>
                    <th className="px-4 py-3 text-left text-sm font-medium text-muted-foreground">Payload</th>
                  </tr>
                </thead>
                <tbody>
                  {messagesQuery.data.items.map((item) => (
                    <tr key={messageKey(item)} className="border-b last:border-0 align-top">
                      <td className="px-4 py-3 text-sm font-mono">{item.msg_id}</td>
                      <td className="px-4 py-3">
                        <QueueStateBadge state={item.state} />
                      </td>
                      <td className="px-4 py-3 text-sm">{item.read_ct}</td>
                      <td className="px-4 py-3 text-sm text-muted-foreground whitespace-nowrap">
                        {formatDate(item.enqueued_at)}
                      </td>
                      <td className="px-4 py-3 text-sm text-muted-foreground whitespace-nowrap">
                        {item.state === "archived"
                          ? `archived: ${formatDate(item.archived_at)}`
                          : `vt: ${formatDate(item.vt)}`}
                      </td>
                      <td className="px-4 py-3 text-sm">
                        <p className="font-medium">{truncate(item.payload.subject as string | null | undefined, 64)}</p>
                        <p className="text-muted-foreground">{truncate(item.payload.body as string | null | undefined, 120)}</p>
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
