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
  PipelineDiscoveredJob,
  DiscoveredJobsStats,
} from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  ChevronLeft,
  ChevronRight,
  ExternalLink,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PAGE_SIZE = 20;

const ACTION_OPTIONS = [
  { value: "all", label: "All Actions" },
  { value: "discovered", label: "Discovered" },
  { value: "notified", label: "Notified" },
  { value: "applied", label: "Applied" },
  { value: "skipped", label: "Skipped" },
] as const;

const SORT_OPTIONS = [
  { value: "created_at", label: "Date" },
  { value: "score", label: "Score" },
] as const;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

// ---------------------------------------------------------------------------
// Score Badge
// ---------------------------------------------------------------------------

function ScoreBadge({ score }: { score: number | null }) {
  if (score == null) {
    return <span className="text-muted-foreground">--</span>;
  }

  let className: string;
  if (score >= 70) {
    className = "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200";
  } else if (score >= 40) {
    className = "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200";
  } else {
    className = "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200";
  }

  return (
    <Badge className={`${className} border-transparent text-xs`}>
      {score}
    </Badge>
  );
}

// ---------------------------------------------------------------------------
// Action Badge
// ---------------------------------------------------------------------------

function ActionBadge({ action }: { action: PipelineDiscoveredJob["action"] }) {
  switch (action) {
    case "Applied":
      return (
        <Badge className="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200 border-transparent text-xs">
          Applied
        </Badge>
      );
    case "Notified":
      return <Badge variant="secondary" className="text-xs">Notified</Badge>;
    case "Skipped":
      return (
        <Badge className="bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300 border-transparent text-xs">
          Skipped
        </Badge>
      );
    default:
      return <Badge variant="outline" className="text-xs">Discovered</Badge>;
  }
}

// ---------------------------------------------------------------------------
// Stat Cards
// ---------------------------------------------------------------------------

function StatCards({ stats }: { stats: DiscoveredJobsStats }) {
  const cards = [
    { label: "Total", value: stats.total },
    { label: "Scored", value: stats.scored_count },
    { label: "Notified", value: stats.by_action.notified },
    { label: "Applied", value: stats.by_action.applied },
    { label: "Skipped", value: stats.by_action.skipped },
    {
      label: "Avg Score",
      value: stats.avg_score != null ? stats.avg_score.toFixed(1) : "--",
    },
  ];

  return (
    <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-6 gap-3">
      {cards.map((c) => (
        <Card key={c.label} className="py-0">
          <CardHeader className="px-4 pt-3 pb-1">
            <CardTitle className="text-xs font-medium text-muted-foreground">
              {c.label}
            </CardTitle>
          </CardHeader>
          <CardContent className="px-4 pb-3">
            <p className="text-xl font-bold">{c.value}</p>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Action Dropdown (inline)
// ---------------------------------------------------------------------------

function ActionDropdown({
  job,
  onUpdate,
  isPending,
}: {
  job: PipelineDiscoveredJob;
  onUpdate: (id: string, action: string) => void;
  isPending: boolean;
}) {
  const actions = ["discovered", "notified", "applied", "skipped"].filter(
    (a) => a !== job.action.toLowerCase(),
  );

  return (
    <Select
      value=""
      onValueChange={(val) => onUpdate(job.id, val)}
      disabled={isPending}
    >
      <SelectTrigger className="h-7 w-[120px] text-xs">
        <SelectValue placeholder="Change..." />
      </SelectTrigger>
      <SelectContent>
        {actions.map((a) => (
          <SelectItem key={a} value={a} className="text-xs">
            Mark as {a.charAt(0).toUpperCase() + a.slice(1)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

// ---------------------------------------------------------------------------
// Loading Skeleton
// ---------------------------------------------------------------------------

function TableSkeleton() {
  return (
    <div className="space-y-2">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className="flex gap-4 items-center">
          <Skeleton className="h-5 flex-1" />
          <Skeleton className="h-5 w-24" />
          <Skeleton className="h-5 w-16" />
          <Skeleton className="h-5 w-20" />
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Page
// ---------------------------------------------------------------------------

export default function DiscoveredJobs() {
  const queryClient = useQueryClient();
  const [actionFilter, setActionFilter] = useState("all");
  const [sortBy, setSortBy] = useState("created_at");
  const [page, setPage] = useState(0);

  const actionParam = actionFilter === "all" ? undefined : actionFilter;

  const { data, isLoading, isError, error } = useQuery({
    queryKey: ["discovered-jobs", actionParam, sortBy, page],
    queryFn: () =>
      api.fetchDiscoveredJobs({
        action: actionParam,
        sort_by: sortBy,
        limit: PAGE_SIZE,
        offset: page * PAGE_SIZE,
      }),
  });

  const { data: stats } = useQuery({
    queryKey: ["discovered-jobs-stats"],
    queryFn: () => api.fetchDiscoveredJobsStats(),
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, action }: { id: string; action: string }) =>
      api.updateDiscoveredJobAction(id, action),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["discovered-jobs"] });
      queryClient.invalidateQueries({ queryKey: ["discovered-jobs-stats"] });
    },
  });

  const items = data?.items ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.ceil(total / PAGE_SIZE);

  return (
    <div className="space-y-6">
      {/* Header */}
      <div>
        <h1 className="text-2xl font-bold">Discovered Jobs</h1>
        <p className="text-muted-foreground mt-1">
          Jobs found across all pipeline runs
        </p>
      </div>

      {/* Stats */}
      {stats && <StatCards stats={stats} />}

      <Separator />

      {/* Filters */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">Action:</span>
          <Select
            value={actionFilter}
            onValueChange={(val) => {
              setActionFilter(val);
              setPage(0);
            }}
          >
            <SelectTrigger className="w-[140px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ACTION_OPTIONS.map((o) => (
                <SelectItem key={o.value} value={o.value}>
                  {o.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">Sort:</span>
          <Select
            value={sortBy}
            onValueChange={(val) => {
              setSortBy(val);
              setPage(0);
            }}
          >
            <SelectTrigger className="w-[110px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {SORT_OPTIONS.map((o) => (
                <SelectItem key={o.value} value={o.value}>
                  {o.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <span className="text-sm text-muted-foreground ml-auto">
          {total} result{total !== 1 ? "s" : ""}
        </span>
      </div>

      {/* Table */}
      {isLoading && <TableSkeleton />}

      {isError && (
        <div className="rounded-lg border border-destructive/50 p-4 text-sm text-destructive">
          Failed to load discovered jobs: {(error as Error).message}
        </div>
      )}

      {!isLoading && !isError && items.length === 0 && (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
          <p className="text-lg font-medium">No discovered jobs yet</p>
          <p className="text-sm text-muted-foreground mt-1">
            Run the pipeline to discover jobs.
          </p>
        </div>
      )}

      {items.length > 0 && (
        <div className="overflow-x-auto rounded border">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-3 py-2 text-left font-medium">Title / Company</th>
                <th className="px-3 py-2 text-left font-medium hidden md:table-cell">
                  Location
                </th>
                <th className="px-3 py-2 text-center font-medium">Score</th>
                <th className="px-3 py-2 text-center font-medium">Action</th>
                <th className="px-3 py-2 text-left font-medium hidden sm:table-cell">
                  Date
                </th>
                <th className="px-3 py-2 text-center font-medium">Update</th>
              </tr>
            </thead>
            <tbody>
              {items.map((job) => (
                <tr
                  key={job.id}
                  className="border-b last:border-b-0 hover:bg-muted/30"
                >
                  <td className="px-3 py-2 max-w-[280px]">
                    <div className="truncate font-medium">
                      {job.url ? (
                        <a
                          href={job.url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="hover:underline inline-flex items-center gap-1"
                        >
                          {job.title}
                          <ExternalLink className="h-3 w-3 shrink-0 text-muted-foreground" />
                        </a>
                      ) : (
                        job.title
                      )}
                    </div>
                    {job.company && (
                      <div className="text-xs text-muted-foreground truncate">
                        {job.company}
                      </div>
                    )}
                  </td>
                  <td className="px-3 py-2 text-muted-foreground hidden md:table-cell">
                    {job.location ?? "--"}
                  </td>
                  <td className="px-3 py-2 text-center">
                    <ScoreBadge score={job.score} />
                  </td>
                  <td className="px-3 py-2 text-center">
                    <ActionBadge action={job.action} />
                  </td>
                  <td className="px-3 py-2 text-xs text-muted-foreground hidden sm:table-cell">
                    {formatDate(job.created_at)}
                  </td>
                  <td className="px-3 py-2 text-center">
                    <ActionDropdown
                      job={job}
                      onUpdate={(id, action) =>
                        updateMutation.mutate({ id, action })
                      }
                      isPending={updateMutation.isPending}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Pagination */}
      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={page === 0}
            onClick={() => setPage((p) => p - 1)}
          >
            <ChevronLeft className="h-4 w-4" />
            Prev
          </Button>
          <span className="text-sm text-muted-foreground">
            Page {page + 1} of {totalPages}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={page >= totalPages - 1}
            onClick={() => setPage((p) => p + 1)}
          >
            Next
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      )}
    </div>
  );
}
