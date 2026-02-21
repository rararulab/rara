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
    <div>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">Coding Tasks</h1>
          <p className="text-muted-foreground text-sm mt-1">
            Dispatch and manage CLI agent tasks
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => queryClient.invalidateQueries({ queryKey: ['coding-tasks'] })}
          >
            <RefreshCw className="h-4 w-4 mr-1" /> Refresh
          </Button>
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4 mr-1" /> New Task
          </Button>
        </div>
      </div>

      {isLoading && <TaskListSkeleton />}
      {isError && (
        <div className="border border-red-300 rounded-lg p-4 text-red-600 text-sm">
          Failed to load tasks: {(error as Error)?.message ?? 'Unknown error'}
        </div>
      )}

      {tasks && tasks.length === 0 && (
        <div className="border rounded-lg p-12 text-center text-muted-foreground">
          <Terminal className="h-12 w-12 mx-auto mb-4 opacity-40" />
          <p className="text-lg font-medium">No coding tasks yet</p>
          <p className="text-sm mt-1">Click "New Task" to dispatch your first coding agent.</p>
        </div>
      )}

      {tasks && tasks.length > 0 && (
        <div className="space-y-2">
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
    <Card>
      <div
        className="flex items-center gap-3 px-4 py-3 cursor-pointer hover:bg-accent/50 transition-colors"
        onClick={onToggle}
      >
        {expanded ? (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        <StatusBadge status={task.status} />
        <Badge variant="secondary" className="font-mono text-xs">
          {task.agent_type}
        </Badge>
        <span className="text-sm flex-1 truncate">{task.prompt}</span>
        <span className="text-xs text-muted-foreground font-mono">{task.id.slice(0, 8)}</span>
        <div className="flex items-center gap-1 text-xs text-muted-foreground">
          <GitBranch className="h-3 w-3" />
          <span className="font-mono">{task.branch}</span>
        </div>
        {task.pr_url && (
          <a
            href={task.pr_url}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(e) => e.stopPropagation()}
            className="text-blue-500 hover:text-blue-600"
          >
            <ExternalLink className="h-4 w-4" />
          </a>
        )}
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
    <div className="border-t px-4 py-4 space-y-4">
      <div className="grid grid-cols-2 gap-4 text-sm">
        <div>
          <span className="text-muted-foreground">Repo:</span>{' '}
          <span className="font-mono text-xs">{task.repo_url}</span>
        </div>
        <div>
          <span className="text-muted-foreground">Tmux:</span>{' '}
          <code className="bg-muted px-1 rounded text-xs">{task.tmux_session}</code>
        </div>
        <div>
          <span className="text-muted-foreground">Created:</span> {formatDate(task.created_at)}
        </div>
        <div>
          <span className="text-muted-foreground">Started:</span> {formatDate(task.started_at)}
        </div>
        <div>
          <span className="text-muted-foreground">Completed:</span> {formatDate(task.completed_at)}
        </div>
        {task.exit_code !== null && (
          <div>
            <span className="text-muted-foreground">Exit code:</span> {task.exit_code}
          </div>
        )}
      </div>

      <div>
        <span className="text-sm text-muted-foreground">Prompt:</span>
        <p className="text-sm mt-1 bg-muted rounded p-2 whitespace-pre-wrap">{task.prompt}</p>
      </div>

      {task.error && (
        <div>
          <span className="text-sm text-red-500 font-medium">Error:</span>
          <pre className="text-xs mt-1 bg-red-50 dark:bg-red-950 rounded p-2 overflow-auto max-h-40">
            {task.error}
          </pre>
        </div>
      )}

      {task.output && (
        <div>
          <span className="text-sm text-muted-foreground">Output (tail):</span>
          <pre className="text-xs mt-1 bg-muted rounded p-2 overflow-auto max-h-60 font-mono">
            {task.output}
          </pre>
        </div>
      )}

      {task.pr_url && (
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">PR:</span>
          <a
            href={task.pr_url}
            target="_blank"
            rel="noopener noreferrer"
            className="text-sm text-blue-500 hover:underline flex items-center gap-1"
          >
            {task.pr_url} <ExternalLink className="h-3 w-3" />
          </a>
        </div>
      )}

      <div className="flex gap-2 pt-2">
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
    <div className="space-y-2">
      {[...Array(3)].map((_, i) => (
        <Skeleton key={i} className="h-14 w-full rounded-lg" />
      ))}
    </div>
  );
}
