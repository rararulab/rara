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

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { CalendarClock, History, Play, Trash2 } from 'lucide-react';
import { useState } from 'react';

import { api } from '@/api/client';
import type { Job, JobResult, Trigger } from '@/api/types';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { Separator } from '@/components/ui/separator';
import { Skeleton } from '@/components/ui/skeleton';

const JOBS_KEY = ['scheduler', 'jobs'] as const;

function formatDate(dateStr: string | null): string {
  if (!dateStr) return '—';
  const d = new Date(dateStr);
  if (Number.isNaN(d.getTime())) return dateStr;
  return d.toLocaleString();
}

/** Collapse `session_key` to its 8-char prefix for display. */
function shortSession(key: string): string {
  if (key.length <= 8) return key;
  return `${key.slice(0, 8)}…`;
}

/** Human-readable summary of a trigger. See the kernel `Trigger` enum. */
function triggerLabel(trigger: Trigger): { text: string; code?: string } {
  switch (trigger.type) {
    case 'once':
      return { text: `Once at ${formatDate(trigger.run_at)}` };
    case 'interval': {
      const s = trigger.every_secs;
      if (s >= 3600 && s % 3600 === 0) return { text: `Every ${s / 3600}h` };
      if (s >= 60 && s % 60 === 0) return { text: `Every ${s / 60}m` };
      return { text: `Every ${s}s` };
    }
    case 'cron':
      return { text: 'Cron', code: trigger.expr };
  }
}

/** `next_at` is only meaningful for recurring / upcoming triggers. */
function triggerNextAt(trigger: Trigger): string | null {
  switch (trigger.type) {
    case 'once':
      return trigger.run_at;
    case 'interval':
    case 'cron':
      return trigger.next_at;
  }
}

function RunStatusBadge({ status }: { status: string | null }) {
  if (!status) return null;
  switch (status) {
    case 'ok':
      return <Badge className="bg-green-100 text-green-800 hover:bg-green-100">ok</Badge>;
    case 'failed':
      return <Badge variant="destructive">failed</Badge>;
    case 'running':
      return <Badge className="bg-blue-100 text-blue-800 hover:bg-blue-100">running</Badge>;
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

function JobHistory({ jobId }: { jobId: string }) {
  const historyQuery = useQuery({
    queryKey: ['scheduler', 'history', jobId],
    queryFn: () => api.get<JobResult[]>(`/api/v1/scheduler/jobs/${jobId}/history?limit=50`),
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
    return <div className="py-6 text-center text-sm text-muted-foreground">No runs yet.</div>;
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
              Completed at
            </th>
            <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
              Summary
            </th>
            <th className="px-4 py-2 text-left text-sm font-medium text-muted-foreground">
              Action taken
            </th>
          </tr>
        </thead>
        <tbody>
          {historyQuery.data.map((run) => (
            <tr
              key={`${run.job_id}-${run.task_id}-${run.completed_at}`}
              className="border-b last:border-0"
            >
              <td className="px-4 py-2">
                <RunStatusBadge status={statusToLabel(run.status)} />
              </td>
              <td className="px-4 py-2 text-sm text-muted-foreground whitespace-nowrap">
                {formatDate(run.completed_at)}
              </td>
              <td className="max-w-xs px-4 py-2 text-sm">
                <span className="line-clamp-2" title={run.summary}>
                  {run.summary || '—'}
                </span>
              </td>
              <td className="px-4 py-2 text-sm">
                {run.action_taken ? (
                  <Badge variant="outline">{run.action_taken}</Badge>
                ) : (
                  <span className="text-muted-foreground">—</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Map the raw kernel `TaskReportStatus` string on `JobResult.status`
 *  onto the same label space used by `Job.last_status`. The admin DTO
 *  already normalises `Job.last_status`, but history rows carry the raw
 *  kernel status (`Completed` / `Failed` / `NeedsApproval`) — we keep the
 *  mapping consistent with `backend-admin/src/scheduler/dto.rs`. */
function statusToLabel(status: string): string {
  switch (status) {
    case 'Completed':
      return 'ok';
    case 'Failed':
      return 'failed';
    case 'NeedsApproval':
      return 'running';
    default:
      return status;
  }
}

function JobCard({ job }: { job: Job }) {
  const queryClient = useQueryClient();
  const [showHistory, setShowHistory] = useState(false);
  const [expandMessage, setExpandMessage] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const triggerMutation = useMutation({
    mutationFn: () => api.post<Job>(`/api/v1/scheduler/jobs/${job.id}/trigger`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: JOBS_KEY });
      void queryClient.invalidateQueries({ queryKey: ['scheduler', 'history', job.id] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => api.del<void>(`/api/v1/scheduler/jobs/${job.id}`),
    onSuccess: () => {
      setConfirmDelete(false);
      void queryClient.invalidateQueries({ queryKey: JOBS_KEY });
    },
  });

  const label = triggerLabel(job.trigger);
  const nextAt = triggerNextAt(job.trigger);

  return (
    <Card>
      <CardContent className="space-y-4 p-6">
        {/* Prompt — the primary visual. Tells the user what the agent plans to do. */}
        <button
          type="button"
          onClick={() => setExpandMessage((v) => !v)}
          className="block w-full text-left"
          aria-label={expandMessage ? 'Collapse prompt' : 'Expand prompt'}
        >
          <p
            className={
              expandMessage
                ? 'whitespace-pre-wrap text-base font-medium leading-relaxed'
                : 'line-clamp-2 text-base font-medium leading-relaxed'
            }
          >
            {job.message}
          </p>
        </button>

        <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
          <CalendarClock className="h-3.5 w-3.5" />
          <span>{label.text}</span>
          {label.code ? (
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs">{label.code}</code>
          ) : null}
          <RunStatusBadge status={job.last_status} />
        </div>

        <Separator />

        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <span className="text-muted-foreground">Last run</span>
            <div className="font-medium">{formatDate(job.last_run_at)}</div>
          </div>
          <div>
            <span className="text-muted-foreground">Next run</span>
            <div className="font-medium">{formatDate(nextAt)}</div>
          </div>
        </div>

        {job.tags.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {job.tags.map((tag) => (
              <Badge key={tag} variant="outline" className="text-xs">
                {tag}
              </Badge>
            ))}
          </div>
        )}

        <div className="font-mono text-xs text-muted-foreground" title={job.session_key}>
          session {shortSession(job.session_key)}
        </div>

        <div className="flex flex-wrap gap-2 pt-1">
          <Button
            variant="outline"
            size="sm"
            onClick={() => triggerMutation.mutate()}
            disabled={triggerMutation.isPending}
          >
            <Play className="mr-1 h-3 w-3" />
            Run now
          </Button>
          <Button variant="outline" size="sm" onClick={() => setShowHistory((v) => !v)}>
            <History className="mr-1 h-3 w-3" />
            {showHistory ? 'Hide history' : 'History'}
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={() => setConfirmDelete(true)}
            disabled={deleteMutation.isPending}
          >
            <Trash2 className="mr-1 h-3 w-3" />
            Delete
          </Button>
        </div>

        {showHistory && (
          <div className="mt-2 rounded-md border">
            <JobHistory jobId={job.id} />
          </div>
        )}
      </CardContent>

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete scheduled job?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes the job from the agent's schedule. History is preserved but the job will
              no longer fire. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteMutation.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault();
                deleteMutation.mutate();
              }}
              disabled={deleteMutation.isPending}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  );
}

function JobsGridSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-[repeat(auto-fill,minmax(360px,1fr))]">
      {Array.from({ length: 3 }).map((_, i) => (
        <Card key={i}>
          <CardContent className="space-y-4 p-6">
            <Skeleton className="h-5 w-full" />
            <Skeleton className="h-4 w-3/4" />
            <Skeleton className="h-4 w-1/2" />
            <Separator />
            <div className="grid grid-cols-2 gap-4">
              <Skeleton className="h-4 w-24" />
              <Skeleton className="h-4 w-24" />
            </div>
            <div className="flex gap-2">
              <Skeleton className="h-8 w-20" />
              <Skeleton className="h-8 w-20" />
              <Skeleton className="h-8 w-20" />
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

/** Scheduler page — lists jobs the agent has scheduled for itself, with
 *  run-now, history, and delete controls. The agent creates jobs via its
 *  own tools; this panel is the user's trust-and-verify surface. */
export default function Scheduler() {
  const jobsQuery = useQuery({
    queryKey: JOBS_KEY,
    queryFn: () => api.get<Job[]>('/api/v1/scheduler/jobs'),
  });

  return (
    <div className="space-y-4">
      <div>
        <div className="flex items-center gap-2">
          <CalendarClock className="h-5 w-5 text-muted-foreground" />
          <h2 className="text-xl font-bold">Scheduler</h2>
        </div>
        <p className="mt-1 text-muted-foreground">Tasks the agent has scheduled for itself.</p>
      </div>

      {jobsQuery.isLoading ? (
        <JobsGridSkeleton />
      ) : !jobsQuery.data?.length ? (
        <Card className="empty-state-card">
          <CardContent className="flex flex-col items-center justify-center py-10 text-muted-foreground">
            <CalendarClock className="mb-3 h-10 w-10 opacity-30" />
            <p className="text-base">
              No scheduled tasks — your agent hasn't scheduled anything yet.
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-[repeat(auto-fill,minmax(360px,1fr))]">
          {jobsQuery.data.map((job) => (
            <JobCard key={job.id} job={job} />
          ))}
        </div>
      )}
    </div>
  );
}
