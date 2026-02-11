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
  SavedJob,
  SavedJobStatus,
  PipelineEvent,
  PipelineEventKind,
} from "@/api/types";
import { SAVED_JOB_STATUSES } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Plus,
  Trash2,
  ExternalLink,
  RotateCcw,
  Loader2,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

const STATUS_LABELS: Record<SavedJobStatus, string> = {
  pending_crawl: "Pending Crawl",
  crawling: "Crawling",
  crawled: "Crawled",
  analyzing: "Analyzing",
  analyzed: "Analyzed",
  failed: "Failed",
  expired: "Expired",
};

function statusBadgeClass(status: string): string {
  switch (status) {
    case "pending_crawl":
    case "crawling":
    case "analyzing":
      return "border-transparent bg-blue-500 text-white";
    case "crawled":
      return "border-transparent bg-yellow-500 text-white";
    case "analyzed":
      return "border-transparent bg-green-600 text-white";
    case "failed":
      return "border-transparent bg-red-500 text-white";
    case "expired":
      return "border-transparent bg-gray-400 text-white";
    default:
      return "";
  }
}

const PROCESSING_STATUSES = new Set(["pending_crawl", "crawling", "analyzing"]);

function StatusBadge({ status }: { status: string }) {
  const isProcessing = PROCESSING_STATUSES.has(status);
  return (
    <Badge className={statusBadgeClass(status)}>
      {isProcessing && <Loader2 className="mr-1 h-3 w-3 animate-spin" />}
      {STATUS_LABELS[status as SavedJobStatus] ?? status}
    </Badge>
  );
}

function formatDate(dateStr: string | null): string {
  if (!dateStr) return "-";
  return new Date(dateStr).toLocaleDateString();
}

function formatDateTime(dateStr: string): string {
  const d = new Date(dateStr);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function truncateUrl(url: string, max = 60): string {
  if (url.length <= max) return url;
  return url.slice(0, max) + "...";
}

// ---------------------------------------------------------------------------
// Create Dialog
// ---------------------------------------------------------------------------

function CreateDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [url, setUrl] = useState("");

  const mutation = useMutation({
    mutationFn: (jobUrl: string) =>
      api.post<SavedJob>("/api/v1/saved-jobs", { url: jobUrl }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["saved-jobs"] });
      setUrl("");
      onOpenChange(false);
    },
  });

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (url.trim()) mutation.mutate(url.trim());
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>Save Job URL</DialogTitle>
          <DialogDescription>
            Paste a job posting URL. It will be crawled and analyzed
            automatically.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="job-url">Job URL *</Label>
            <Input
              id="job-url"
              type="url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://..."
              required
            />
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? "Saving..." : "Save"}
            </Button>
          </DialogFooter>
          {mutation.isError && (
            <p className="text-sm text-destructive">
              Error: {(mutation.error as Error).message}
            </p>
          )}
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Delete Confirmation Dialog
// ---------------------------------------------------------------------------

function DeleteDialog({
  open,
  onOpenChange,
  onDeleted,
  job,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDeleted: () => void;
  job: SavedJob;
}) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => api.del<void>(`/api/v1/saved-jobs/${job.id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["saved-jobs"] });
      onDeleted();
    },
  });

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent className="sm:max-w-[400px]">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete Saved Job</AlertDialogTitle>
          <AlertDialogDescription>
            Are you sure you want to delete{" "}
            <strong>{job.title ?? truncateUrl(job.url)}</strong>? This action
            cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {mutation.isError && (
          <p className="text-sm text-destructive">
            Error: {(mutation.error as Error).message}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={mutation.isPending}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            onClick={(e) => {
              e.preventDefault();
              mutation.mutate();
            }}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? "Deleting..." : "Delete"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

// ---------------------------------------------------------------------------
// Pipeline Timeline
// ---------------------------------------------------------------------------

const EVENT_KIND_COLORS: Record<PipelineEventKind, string> = {
  started: "bg-blue-500",
  completed: "bg-green-500",
  failed: "bg-red-500",
  info: "bg-gray-400",
};

const STAGE_LABELS: Record<string, string> = {
  crawl: "Crawl",
  analyze: "Analyze",
  gc: "GC",
};

function PipelineTimeline({ events }: { events: PipelineEvent[] }) {
  if (events.length === 0) {
    return (
      <p className="text-sm text-muted-foreground italic">
        No pipeline events yet.
      </p>
    );
  }

  return (
    <div className="relative space-y-0">
      {events.map((event, idx) => {
        const dotColor =
          EVENT_KIND_COLORS[event.event_kind as PipelineEventKind] ??
          "bg-gray-400";
        const isLast = idx === events.length - 1;

        return (
          <div key={event.id} className="flex gap-3 relative">
            {/* Vertical line + dot */}
            <div className="flex flex-col items-center">
              <div
                className={`w-2.5 h-2.5 rounded-full shrink-0 mt-1.5 ${dotColor}`}
              />
              {!isLast && (
                <div className="w-px flex-1 bg-border min-h-[24px]" />
              )}
            </div>

            {/* Content */}
            <div className="pb-4 min-w-0 flex-1">
              <div className="flex items-center gap-2 flex-wrap">
                <Badge variant="outline" className="text-xs px-1.5 py-0">
                  {STAGE_LABELS[event.stage] ?? event.stage}
                </Badge>
                <span className="text-xs text-muted-foreground">
                  {formatDateTime(event.created_at)}
                </span>
              </div>
              <p className="text-sm mt-0.5">{event.message}</p>
              {event.metadata && (
                <div className="mt-1 text-xs text-muted-foreground font-mono bg-muted/50 rounded px-2 py-1 inline-block">
                  {Object.entries(event.metadata).map(([k, v]) => (
                    <span key={k} className="mr-3">
                      {k}={String(v)}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Analysis Details (used in modal)
// ---------------------------------------------------------------------------

function AnalysisDetails({ job }: { job: SavedJob }) {
  const analysis = job.analysis_result as Record<string, unknown> | null;

  if (!analysis && !job.error_message) return null;

  return (
    <div className="space-y-3">
      {/* Analysis Result Fields */}
      {analysis && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
          {"summary" in analysis && (
            <div className="md:col-span-2">
              <span className="font-medium">Summary:</span>
              <p className="text-muted-foreground mt-0.5">
                {String(analysis.summary)}
              </p>
            </div>
          )}
          {"required_skills" in analysis && (
            <div>
              <span className="font-medium">Required Skills:</span>
              <p className="text-muted-foreground mt-0.5">
                {Array.isArray(analysis.required_skills)
                  ? (analysis.required_skills as string[]).join(", ")
                  : String(analysis.required_skills)}
              </p>
            </div>
          )}
          {"experience_level" in analysis && (
            <div>
              <span className="font-medium">Experience Level:</span>
              <p className="text-muted-foreground mt-0.5">
                {String(analysis.experience_level)}
              </p>
            </div>
          )}
          {"salary_range" in analysis && (
            <div>
              <span className="font-medium">Salary Range:</span>
              <p className="text-muted-foreground mt-0.5">
                {String(analysis.salary_range)}
              </p>
            </div>
          )}
        </div>
      )}

      {/* Error Message */}
      {job.error_message && (
        <div className="text-sm text-red-500">
          <span className="font-medium">Error:</span> {job.error_message}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Saved Job Detail Modal
// ---------------------------------------------------------------------------

function SavedJobDetailModal({
  job,
  open,
  onOpenChange,
  onRetry,
  onDelete,
}: {
  job: SavedJob;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRetry: () => void;
  onDelete: () => void;
}) {
  const isProcessing = PROCESSING_STATUSES.has(job.status);
  const canRetry = job.status === "failed" || job.status === "expired";

  const {
    data: events,
    isLoading: eventsLoading,
  } = useQuery({
    queryKey: ["saved-job-events", job.id],
    queryFn: () =>
      api.get<PipelineEvent[]>(`/api/v1/saved-jobs/${job.id}/events`),
    refetchInterval: isProcessing ? 3000 : false,
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl h-[85vh] overflow-hidden flex flex-col p-0">
        {/* Header */}
        <DialogHeader className="px-6 pt-6 pb-4 pr-14 shrink-0 border-b">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <DialogTitle className="text-lg font-semibold truncate">
                {job.title ?? truncateUrl(job.url, 80)}
              </DialogTitle>
              <DialogDescription className="mt-1">
                {job.company && (
                  <span className="font-medium">{job.company}</span>
                )}
                {job.company && " -- "}
                <span className="text-xs">{truncateUrl(job.url, 70)}</span>
              </DialogDescription>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              {job.match_score != null && (
                <span className="text-sm font-bold">{job.match_score}%</span>
              )}
              <StatusBadge status={job.status} />
            </div>
          </div>
        </DialogHeader>

        {/* Scrollable content */}
        <div className="flex-1 overflow-y-auto px-6 py-4 space-y-6">
          {/* Match Score Bar */}
          {job.match_score != null && (
            <div className="space-y-1">
              <div className="flex items-center gap-2 text-sm">
                <span className="font-medium">Match Score:</span>
                <span className="font-bold text-lg">{job.match_score}%</span>
              </div>
              <div className="h-2 w-full max-w-xs rounded-full bg-muted overflow-hidden">
                <div
                  className="h-full rounded-full bg-green-500 transition-all"
                  style={{ width: `${Math.min(job.match_score, 100)}%` }}
                />
              </div>
            </div>
          )}

          {/* Analysis Details */}
          <AnalysisDetails job={job} />

          {/* Pipeline Timeline */}
          <div>
            <h3 className="text-sm font-semibold mb-3">Pipeline Timeline</h3>
            {eventsLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-4 w-64" />
                <Skeleton className="h-4 w-48" />
                <Skeleton className="h-4 w-56" />
              </div>
            ) : (
              <PipelineTimeline events={events ?? []} />
            )}
          </div>

          {/* Markdown Preview */}
          {job.markdown_preview && (
            <details className="text-sm">
              <summary className="cursor-pointer font-medium text-muted-foreground hover:text-foreground">
                Markdown Preview
              </summary>
              <pre className="mt-2 max-h-48 overflow-auto rounded bg-muted p-3 text-xs whitespace-pre-wrap">
                {job.markdown_preview}
              </pre>
            </details>
          )}

          {/* Timestamps */}
          <div className="text-xs text-muted-foreground space-y-0.5">
            <p>Created: {formatDate(job.created_at)}</p>
            {job.crawled_at && <p>Crawled: {formatDate(job.crawled_at)}</p>}
            {job.analyzed_at && <p>Analyzed: {formatDate(job.analyzed_at)}</p>}
            {job.expires_at && <p>Expires: {formatDate(job.expires_at)}</p>}
          </div>
        </div>

        {/* Footer */}
        <DialogFooter className="px-6 py-4 shrink-0 border-t">
          <div className="flex items-center gap-2 w-full justify-between">
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => window.open(job.url, "_blank")}
              >
                <ExternalLink className="h-4 w-4 mr-1" />
                Open URL
              </Button>
            </div>
            <div className="flex items-center gap-2">
              {canRetry && (
                <Button variant="outline" size="sm" onClick={onRetry}>
                  <RotateCcw className="h-4 w-4 mr-1" />
                  Retry
                </Button>
              )}
              <Button variant="destructive" size="sm" onClick={onDelete}>
                <Trash2 className="h-4 w-4 mr-1" />
                Delete
              </Button>
            </div>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Job Card
// ---------------------------------------------------------------------------

function JobCard({
  job,
  onSelect,
  onRetry,
  onDelete,
}: {
  job: SavedJob;
  onSelect: () => void;
  onRetry: () => void;
  onDelete: () => void;
}) {
  const canRetry = job.status === "failed" || job.status === "expired";

  return (
    <div className="border rounded-lg overflow-hidden">
      <div
        className="flex items-center gap-3 p-4 hover:bg-muted/30 cursor-pointer transition-colors"
        onClick={onSelect}
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium truncate">
              {job.title ?? truncateUrl(job.url)}
            </span>
            <a
              href={job.url}
              target="_blank"
              rel="noopener noreferrer"
              onClick={(e) => e.stopPropagation()}
              className="text-muted-foreground hover:text-foreground shrink-0"
            >
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          </div>
          {job.company && (
            <p className="text-sm text-muted-foreground">{job.company}</p>
          )}
        </div>

        {job.match_score != null && (
          <span className="text-sm font-medium shrink-0">
            {job.match_score}%
          </span>
        )}

        <StatusBadge status={job.status} />

        <span className="text-xs text-muted-foreground shrink-0 hidden sm:inline">
          {formatDate(job.created_at)}
        </span>

        <div
          className="flex items-center gap-1 shrink-0"
          onClick={(e) => e.stopPropagation()}
        >
          {canRetry && (
            <Button
              variant="ghost"
              size="sm"
              onClick={onRetry}
              title="Retry"
            >
              <RotateCcw className="h-4 w-4" />
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            title="Delete"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Loading Skeleton
// ---------------------------------------------------------------------------

function ListSkeleton() {
  return (
    <div className="space-y-3">
      {Array.from({ length: 4 }).map((_, i) => (
        <div key={i} className="border rounded-lg p-4 flex gap-4">
          <div className="flex-1 space-y-2">
            <Skeleton className="h-5 w-64" />
            <Skeleton className="h-4 w-32" />
          </div>
          <Skeleton className="h-6 w-20" />
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Page
// ---------------------------------------------------------------------------

export default function SavedJobs() {
  const queryClient = useQueryClient();
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteJob, setDeleteJob] = useState<SavedJob | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [selectedJob, setSelectedJob] = useState<SavedJob | null>(null);
  const [statusFilter, setStatusFilter] = useState<string>("all");

  const closeDeleteDialog = () => {
    setDeleteOpen(false);
    // Don't null deleteJob here — let AlertDialog close animation finish.
    // deleteJob will be overwritten next time a delete is initiated.
  };

  const handleDeleteSuccess = () => {
    const deletingId = deleteJob?.id;
    setDeleteOpen(false);
    setDeleteJob(null);
    if (deletingId && selectedJob?.id === deletingId) {
      setSelectedJob(null);
    }
  };

  const openDeleteDialog = (job: SavedJob) => {
    setDeleteJob(job);
    setDeleteOpen(true);
  };

  const filterParam =
    statusFilter === "all" ? "" : `?status=${statusFilter}`;

  const {
    data: jobs,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ["saved-jobs", statusFilter],
    queryFn: () =>
      api.get<SavedJob[]>(`/api/v1/saved-jobs${filterParam}`),
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return false;
      const hasProcessing = data.some((j) => PROCESSING_STATUSES.has(j.status));
      return hasProcessing ? 5000 : false;
    },
  });

  // Keep selectedJob in sync with fresh data from the list query
  const freshSelectedJob =
    selectedJob && jobs
      ? jobs.find((j) => j.id === selectedJob.id) ?? selectedJob
      : selectedJob;

  const retryMutation = useMutation({
    mutationFn: (id: string) =>
      api.post<void>(`/api/v1/saved-jobs/${id}/retry`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["saved-jobs"] });
    },
  });

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold">Saved Jobs</h1>
          <p className="text-muted-foreground mt-1">
            Save job URLs to crawl, analyze, and track.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Select value={statusFilter} onValueChange={setStatusFilter}>
            <SelectTrigger className="w-[160px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All Statuses</SelectItem>
              {SAVED_JOB_STATUSES.map((s) => (
                <SelectItem key={s} value={s}>
                  {STATUS_LABELS[s]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            Save Job URL
          </Button>
        </div>
      </div>

      <Separator />

      {/* Content */}
      {isLoading && <ListSkeleton />}

      {isError && (
        <div className="rounded-lg border border-destructive/50 p-4 text-sm text-destructive">
          Failed to load saved jobs: {(error as Error).message}
        </div>
      )}

      {jobs && jobs.length === 0 && (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
          <p className="text-lg font-medium">No saved jobs yet</p>
          <p className="text-sm text-muted-foreground mt-1">
            Paste a job posting URL to get started.
          </p>
          <Button className="mt-4" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            Save Job URL
          </Button>
        </div>
      )}

      {jobs && jobs.length > 0 && (
        <div className="space-y-2">
          {jobs.map((job) => (
            <JobCard
              key={job.id}
              job={job}
              onSelect={() => setSelectedJob(job)}
              onRetry={() => retryMutation.mutate(job.id)}
              onDelete={() => openDeleteDialog(job)}
            />
          ))}
        </div>
      )}

      {/* Dialogs */}
      {createOpen && (
        <CreateDialog
          open={createOpen}
          onOpenChange={setCreateOpen}
        />
      )}

      {deleteJob && (
        <DeleteDialog
          key={deleteJob.id}
          open={deleteOpen}
          onOpenChange={(open) => {
            if (!open) closeDeleteDialog();
          }}
          onDeleted={handleDeleteSuccess}
          job={deleteJob}
        />
      )}

      {freshSelectedJob && (
        <SavedJobDetailModal
          key={freshSelectedJob.id}
          job={freshSelectedJob}
          open={true}
          onOpenChange={(open) => {
            if (!open) setSelectedJob(null);
          }}
          onRetry={() => {
            retryMutation.mutate(freshSelectedJob.id);
          }}
          onDelete={() => {
            // Close detail modal first, then open delete confirmation.
            // Avoids nesting two Radix dialogs which causes pointer-event conflicts.
            const job = freshSelectedJob;
            setSelectedJob(null);
            openDeleteDialog(job);
          }}
        />
      )}
    </div>
  );
}
