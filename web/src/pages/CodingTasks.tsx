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

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/api/client';
import type {
  CodingTaskSummary,
  CodingTaskDetail,
  CreateCodingTaskRequest,
  CodingTaskStatus,
} from '@/api/types';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Card } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { Skeleton } from '@/components/ui/skeleton';
import {
  Plus,
  RefreshCw,
  GitBranch,
  ExternalLink,
  Terminal,
  Play,
  XCircle,
  GitMerge,
  ChevronDown,
  ChevronRight,
} from 'lucide-react';

const STATUS_CONFIG: Record<CodingTaskStatus, { label: string; variant: string; emoji: string }> = {
  Pending:     { label: 'Pending',      variant: 'bg-gray-100 text-gray-700 dark:bg-gray-800 dark:text-gray-300',   emoji: '\u23F3' },
  Cloning:     { label: 'Cloning',      variant: 'bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300',   emoji: '\uD83D\uDCE6' },
  Running:     { label: 'Running',      variant: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300', emoji: '\uD83C\uDFC3' },
  Completed:   { label: 'Completed',    variant: 'bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300', emoji: '\u2705' },
  Failed:      { label: 'Failed',       variant: 'bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300',       emoji: '\u274C' },
  Merged:      { label: 'Merged',       variant: 'bg-purple-100 text-purple-700 dark:bg-purple-900 dark:text-purple-300', emoji: '\uD83C\uDF89' },
  MergeFailed: { label: 'Merge Failed', variant: 'bg-orange-100 text-orange-700 dark:bg-orange-900 dark:text-orange-300', emoji: '\u26A0\uFE0F' },
};

function StatusBadge({ status }: { status: CodingTaskStatus }) {
  const config = STATUS_CONFIG[status] ?? STATUS_CONFIG.Pending;
  return (
    <Badge variant="outline" className={config.variant}>
      {config.emoji} {config.label}
    </Badge>
  );
}

function formatDate(iso: string | null): string {
  if (!iso) return '\u2014';
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

export default function CodingTasks() {
  const queryClient = useQueryClient();
  const [createOpen, setCreateOpen] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const { data: tasks, isLoading, isError, error } = useQuery({
    queryKey: ['coding-tasks'],
    queryFn: () => api.get<CodingTaskSummary[]>('/api/v1/coding-tasks'),
    refetchInterval: 5000,
  });

  return (
    <div className="space-y-6">
      <div className="data-panel flex flex-col gap-4 p-5 md:flex-row md:items-center md:justify-between md:p-6">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Coding Tasks</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Dispatch and manage CLI agent tasks
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            className="rounded-xl"
            onClick={() => queryClient.invalidateQueries({ queryKey: ['coding-tasks'] })}
          >
            <RefreshCw className="h-4 w-4 mr-1" /> Refresh
          </Button>
          <Button size="sm" className="rounded-xl" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4 mr-1" /> New Task
          </Button>
        </div>
      </div>

      {isLoading && <TaskListSkeleton />}
      {isError && (
        <div className="rounded-2xl border border-destructive/40 bg-destructive/5 p-4 text-sm text-destructive">
          Failed to load tasks: {(error as Error)?.message ?? 'Unknown error'}
        </div>
      )}

      {tasks && tasks.length === 0 && (
        <div className="empty-state-card">
          <Terminal className="h-12 w-12 mx-auto mb-4 opacity-40" />
          <p className="text-lg font-medium">No coding tasks yet</p>
          <p className="text-sm mt-1">Click "New Task" to dispatch your first coding agent.</p>
        </div>
      )}

      {tasks && tasks.length > 0 && (
        <div className="space-y-3">
          {tasks.map((task) => (
            <TaskRow
              key={task.id}
              task={task}
              expanded={expandedId === task.id}
              onToggle={() => setExpandedId(expandedId === task.id ? null : task.id)}
            />
          ))}
        </div>
      )}

      <CreateTaskDialog open={createOpen} onOpenChange={setCreateOpen} />
    </div>
  );
}

function TaskRow({
  task,
  expanded,
  onToggle,
}: {
  task: CodingTaskSummary;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <Card className="data-panel overflow-hidden">
      <div
        className="cursor-pointer px-4 py-4 transition-colors hover:bg-background/45"
        onClick={onToggle}
      >
        <div className="flex flex-col gap-3 md:flex-row md:items-center md:gap-4">
          <div className="flex min-w-0 items-start gap-3">
            {expanded ? (
              <ChevronDown className="mt-1 h-4 w-4 shrink-0 text-muted-foreground" />
            ) : (
              <ChevronRight className="mt-1 h-4 w-4 shrink-0 text-muted-foreground" />
            )}
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm leading-6 md:text-base">{task.prompt}</p>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <StatusBadge status={task.status} />
                <Badge variant="secondary" className="font-mono text-xs">
                  {task.agent_type}
                </Badge>
                <span className="code-chip font-mono">{task.id.slice(0, 8)}</span>
                <span className="inline-flex items-center gap-1 rounded-md border border-border/60 bg-background/50 px-2 py-1 text-xs text-muted-foreground">
                  <GitBranch className="h-3 w-3" />
                  <span className="font-mono">{task.branch}</span>
                </span>
              </div>
            </div>
          </div>
          {task.pr_url && (
            <a
              href={task.pr_url}
              target="_blank"
              rel="noopener noreferrer"
              onClick={(e) => e.stopPropagation()}
              className="self-start rounded-lg border border-border/60 bg-background/70 p-2 text-blue-500 hover:bg-background hover:text-blue-600 md:self-center"
              title="Open PR"
            >
              <ExternalLink className="h-4 w-4" />
            </a>
          )}
        </div>
      </div>
      {expanded && <TaskDetailPanel taskId={task.id} />}
    </Card>
  );
}

function TaskDetailPanel({ taskId }: { taskId: string }) {
  const queryClient = useQueryClient();
  const { data: task, isLoading } = useQuery({
    queryKey: ['coding-task', taskId],
    queryFn: () => api.get<CodingTaskDetail>(`/api/v1/coding-tasks/${taskId}`),
    refetchInterval: 3000,
  });

  const mergeMutation = useMutation({
    mutationFn: () => api.post(`/api/v1/coding-tasks/${taskId}/merge`, {}),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['coding-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['coding-task', taskId] });
    },
  });

  const cancelMutation = useMutation({
    mutationFn: () => api.post(`/api/v1/coding-tasks/${taskId}/cancel`, {}),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['coding-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['coding-task', taskId] });
    },
  });

  if (isLoading || !task) {
    return (
      <div className="px-4 pb-4">
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-4 border-t border-border/60 bg-background/20 px-4 py-4">
      <div className="grid grid-cols-1 gap-3 text-sm md:grid-cols-2">
        <div className="rounded-lg bg-background/50 px-3 py-2">
          <span className="text-muted-foreground">Repo:</span>{' '}
          <span className="font-mono text-xs">{task.repo_url}</span>
        </div>
        <div className="rounded-lg bg-background/50 px-3 py-2">
          <span className="text-muted-foreground">Tmux:</span>{' '}
          <code className="code-chip">{task.tmux_session}</code>
        </div>
        <div className="rounded-lg bg-background/50 px-3 py-2">
          <span className="text-muted-foreground">Created:</span> {formatDate(task.created_at)}
        </div>
        <div className="rounded-lg bg-background/50 px-3 py-2">
          <span className="text-muted-foreground">Started:</span> {formatDate(task.started_at)}
        </div>
        <div className="rounded-lg bg-background/50 px-3 py-2">
          <span className="text-muted-foreground">Completed:</span> {formatDate(task.completed_at)}
        </div>
        {task.exit_code !== null && (
          <div className="rounded-lg bg-background/50 px-3 py-2">
            <span className="text-muted-foreground">Exit code:</span> {task.exit_code}
          </div>
        )}
      </div>

      <div>
        <span className="text-sm text-muted-foreground">Prompt:</span>
        <p className="mt-1 whitespace-pre-wrap rounded-xl border border-border/60 bg-background/60 p-3 text-sm">{task.prompt}</p>
      </div>

      {task.error && (
        <div>
          <span className="text-sm text-red-500 font-medium">Error:</span>
          <pre className="mt-1 max-h-40 overflow-auto rounded-xl border border-destructive/20 bg-red-50 p-3 text-xs dark:bg-red-950">
            {task.error}
          </pre>
        </div>
      )}

      {task.output && (
        <div>
          <span className="text-sm text-muted-foreground">Output (tail):</span>
          <pre className="mt-1 max-h-60 overflow-auto rounded-xl border border-border/60 bg-background/60 p-3 font-mono text-xs">
            {task.output}
          </pre>
        </div>
      )}

      {task.pr_url && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg bg-background/40 px-3 py-2">
          <span className="text-sm text-muted-foreground">PR:</span>
          <a
            href={task.pr_url}
            target="_blank"
            rel="noopener noreferrer"
            className="flex items-center gap-1 text-sm text-blue-500 hover:underline"
          >
            {task.pr_url} <ExternalLink className="h-3 w-3" />
          </a>
        </div>
      )}

      <div className="flex flex-wrap gap-2 pt-2">
        {task.status === 'Completed' && task.pr_url && (
          <Button
            size="sm"
            onClick={() => mergeMutation.mutate()}
            disabled={mergeMutation.isPending}
          >
            <GitMerge className="h-4 w-4 mr-1" />
            {mergeMutation.isPending ? 'Merging...' : 'Merge PR'}
          </Button>
        )}
        {(task.status === 'Running' || task.status === 'Cloning' || task.status === 'Pending') && (
          <Button
            size="sm"
            variant="destructive"
            onClick={() => cancelMutation.mutate()}
            disabled={cancelMutation.isPending}
          >
            <XCircle className="h-4 w-4 mr-1" />
            {cancelMutation.isPending ? 'Cancelling...' : 'Cancel'}
          </Button>
        )}
      </div>
    </div>
  );
}

function CreateTaskDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [prompt, setPrompt] = useState('');
  const [agentType, setAgentType] = useState<'Claude' | 'Codex'>('Claude');
  const [repoUrl, setRepoUrl] = useState('');

  const mutation = useMutation({
    mutationFn: (data: CreateCodingTaskRequest) =>
      api.post<CodingTaskDetail>('/api/v1/coding-tasks', data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['coding-tasks'] });
      onOpenChange(false);
      setPrompt('');
      setRepoUrl('');
    },
  });

  const handleSubmit = () => {
    if (!prompt.trim()) return;
    mutation.mutate({
      prompt: prompt.trim(),
      agent_type: agentType,
      repo_url: repoUrl.trim() || undefined,
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Dispatch Coding Task</DialogTitle>
          <DialogDescription>
            Send a prompt to a CLI agent. It will run in a tmux session with an isolated git worktree.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 py-4">
          <div>
            <Label htmlFor="prompt">Prompt</Label>
            <Textarea
              id="prompt"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Fix the login bug in auth.rs..."
              rows={4}
            />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div>
              <Label>Agent</Label>
              <div className="flex gap-2 mt-1">
                <Button
                  type="button"
                  variant={agentType === 'Claude' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setAgentType('Claude')}
                >
                  Claude
                </Button>
                <Button
                  type="button"
                  variant={agentType === 'Codex' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setAgentType('Codex')}
                >
                  Codex
                </Button>
              </div>
            </div>
            <div>
              <Label htmlFor="repo_url">Repo URL (optional)</Label>
              <Input
                id="repo_url"
                value={repoUrl}
                onChange={(e) => setRepoUrl(e.target.value)}
                placeholder="https://github.com/..."
              />
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!prompt.trim() || mutation.isPending}>
            <Play className="h-4 w-4 mr-1" />
            {mutation.isPending ? 'Dispatching...' : 'Dispatch'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function TaskListSkeleton() {
  return (
    <div className="space-y-3">
      {[...Array(3)].map((_, i) => (
        <Skeleton key={i} className="h-[4.5rem] w-full rounded-xl" />
      ))}
    </div>
  );
}
