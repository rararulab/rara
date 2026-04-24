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
import { Plus, Trash2, X } from 'lucide-react';
import { useMemo, useState } from 'react';

import { api } from '@/api/client';
import type { ChatSession } from '@/api/types';
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';

// ---------------------------------------------------------------------------
// Types — mirrors the Rust `SubscriptionDto`
// ---------------------------------------------------------------------------

type NotifyAction = 'proactive_turn' | 'silent_append';

interface SubscriptionDto {
  id: string;
  subscriber: string;
  owner: string;
  match_tags: string[];
  on_receive: NotifyAction;
}

interface CreateBody {
  subscriber: string;
  owner?: string;
  match_tags: string[];
  on_receive: NotifyAction;
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

/**
 * Subscriptions — admin UI for the kernel notification registry.
 *
 * Operators create subscriptions that bind a chat session to a set of data
 * feed tags. When a matching `FeedEvent` fires, the kernel dispatches a
 * `ProactiveTurn` or appends silently depending on the chosen action.
 */
export default function Subscriptions() {
  const queryClient = useQueryClient();
  const [createOpen, setCreateOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<SubscriptionDto | null>(null);

  const subsQuery = useQuery({
    queryKey: ['subscriptions'],
    queryFn: () => api.get<SubscriptionDto[]>('/api/v1/subscriptions'),
  });

  const sessionsQuery = useQuery({
    queryKey: ['chat-sessions-brief'],
    queryFn: () => api.get<ChatSession[]>('/api/v1/chat/sessions?limit=100&offset=0'),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.del<void>(`/api/v1/subscriptions/${id}`),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ['subscriptions'] }),
  });

  const sessionLabel = useMemo(() => {
    const index = new Map<string, string>();
    for (const s of sessionsQuery.data ?? []) {
      index.set(s.key, s.title ?? s.key.slice(0, 8));
    }
    return (key: string) => index.get(key) ?? key.slice(0, 8);
  }, [sessionsQuery.data]);

  const subs = subsQuery.data ?? [];

  return (
    <div className="flex h-full flex-col gap-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">Subscriptions</h1>
          <p className="text-sm text-muted-foreground">
            Bind a chat session to data feed tags — matching events trigger a proactive turn or get
            silently appended to the conversation.
          </p>
        </div>
        <Button onClick={() => setCreateOpen(true)} className="gap-1.5">
          <Plus className="h-4 w-4" />
          New subscription
        </Button>
      </div>

      {/* Table */}
      <Card className="flex-1 min-h-0 overflow-auto p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Tags</TableHead>
              <TableHead className="w-[160px]">Action</TableHead>
              <TableHead className="w-[200px]">Subscriber session</TableHead>
              <TableHead className="w-[140px]">Owner</TableHead>
              <TableHead className="w-[60px]" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {subsQuery.isLoading && (
              <TableRow>
                <TableCell colSpan={5} className="text-center text-sm text-muted-foreground">
                  Loading…
                </TableCell>
              </TableRow>
            )}
            {!subsQuery.isLoading && subs.length === 0 && (
              <TableRow>
                <TableCell colSpan={5} className="text-center text-sm text-muted-foreground">
                  No subscriptions yet — click “New subscription” to create one.
                </TableCell>
              </TableRow>
            )}
            {subs.map((sub) => (
              <TableRow key={sub.id}>
                <TableCell>
                  <div className="flex flex-wrap gap-1">
                    {sub.match_tags.map((tag) => (
                      <Badge key={tag} variant="secondary">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                </TableCell>
                <TableCell>
                  <Badge variant={sub.on_receive === 'proactive_turn' ? 'default' : 'outline'}>
                    {sub.on_receive}
                  </Badge>
                </TableCell>
                <TableCell>
                  <span className="font-mono text-xs" title={sub.subscriber}>
                    {sessionLabel(sub.subscriber)}
                  </span>
                </TableCell>
                <TableCell className="text-sm">{sub.owner}</TableCell>
                <TableCell>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 text-muted-foreground hover:text-destructive"
                    onClick={() => setPendingDelete(sub)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Card>

      {/* Create dialog */}
      <CreateSubscriptionDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        sessions={sessionsQuery.data ?? []}
        onCreated={() => queryClient.invalidateQueries({ queryKey: ['subscriptions'] })}
      />

      {/* Delete confirm */}
      <AlertDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => !open && setPendingDelete(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this subscription?</AlertDialogTitle>
            <AlertDialogDescription>
              Matching feed events will stop being dispatched to the bound session. This action is
              audited and requires admin role on the backend.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (pendingDelete) {
                  deleteMutation.mutate(pendingDelete.id);
                  setPendingDelete(null);
                }
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Create dialog
// ---------------------------------------------------------------------------

interface CreateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sessions: ChatSession[];
  onCreated: () => void;
}

function CreateSubscriptionDialog({ open, onOpenChange, sessions, onCreated }: CreateDialogProps) {
  const [subscriber, setSubscriber] = useState<string>('');
  const [tagsInput, setTagsInput] = useState<string>('');
  const [tags, setTags] = useState<string[]>([]);
  const [action, setAction] = useState<NotifyAction>('proactive_turn');
  const [error, setError] = useState<string | null>(null);

  const createMutation = useMutation({
    mutationFn: (body: CreateBody) => api.post<SubscriptionDto>('/api/v1/subscriptions', body),
    onSuccess: () => {
      onCreated();
      reset();
      onOpenChange(false);
    },
    onError: (err: Error) => setError(err.message),
  });

  function reset() {
    setSubscriber('');
    setTagsInput('');
    setTags([]);
    setAction('proactive_turn');
    setError(null);
  }

  function commitTagInput() {
    const trimmed = tagsInput.trim();
    if (!trimmed) return;
    if (!tags.includes(trimmed)) {
      setTags([...tags, trimmed]);
    }
    setTagsInput('');
  }

  function submit() {
    setError(null);
    if (!subscriber) {
      setError('Select a subscriber session');
      return;
    }
    const allTags = tagsInput.trim() ? [...tags, tagsInput.trim()] : tags;
    if (allTags.length === 0) {
      setError('At least one tag is required');
      return;
    }
    createMutation.mutate({
      subscriber,
      match_tags: allTags,
      on_receive: action,
    });
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) reset();
        onOpenChange(nextOpen);
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>New subscription</DialogTitle>
          <DialogDescription>
            Route feed events that carry any of the selected tags to the chosen session.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          {/* Subscriber session */}
          <div className="space-y-2">
            <Label htmlFor="subscriber">Subscriber session</Label>
            <Select value={subscriber} onValueChange={setSubscriber}>
              <SelectTrigger id="subscriber">
                <SelectValue placeholder="Select a session…" />
              </SelectTrigger>
              <SelectContent>
                {sessions.length === 0 && (
                  <SelectItem value="__none" disabled>
                    No sessions available
                  </SelectItem>
                )}
                {sessions.map((s) => (
                  <SelectItem key={s.key} value={s.key}>
                    {s.title ?? s.key.slice(0, 8)}
                    <span className="ml-2 font-mono text-xs text-muted-foreground">
                      {s.key.slice(0, 8)}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Tags */}
          <div className="space-y-2">
            <Label htmlFor="tags">Match tags</Label>
            <div className="flex flex-wrap gap-1 rounded-md border border-input bg-background p-2">
              {tags.map((tag) => (
                <Badge key={tag} variant="secondary" className="gap-1 pr-1">
                  {tag}
                  <button
                    type="button"
                    onClick={() => setTags(tags.filter((t) => t !== tag))}
                    className="rounded-sm hover:bg-muted-foreground/20"
                  >
                    <X className="h-3 w-3" />
                  </button>
                </Badge>
              ))}
              <Input
                id="tags"
                value={tagsInput}
                onChange={(e) => setTagsInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ',') {
                    e.preventDefault();
                    commitTagInput();
                  } else if (e.key === 'Backspace' && !tagsInput && tags.length > 0) {
                    setTags(tags.slice(0, -1));
                  }
                }}
                onBlur={commitTagInput}
                placeholder="e.g. news.aapl (press Enter)"
                className="flex-1 min-w-[120px] border-0 bg-transparent p-0 shadow-none focus-visible:ring-0"
              />
            </div>
            <p className="text-xs text-muted-foreground">
              Press Enter or comma to add a tag. A feed event whose tag list intersects any of these
              will fire the subscription.
            </p>
          </div>

          {/* Action */}
          <div className="space-y-2">
            <Label>On receive</Label>
            <div className="flex gap-4 text-sm">
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  name="on_receive"
                  value="proactive_turn"
                  checked={action === 'proactive_turn'}
                  onChange={() => setAction('proactive_turn')}
                />
                <span>
                  <span className="font-medium">proactive_turn</span> — kick the agent into a new
                  turn
                </span>
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  name="on_receive"
                  value="silent_append"
                  checked={action === 'silent_append'}
                  onChange={() => setAction('silent_append')}
                />
                <span>
                  <span className="font-medium">silent_append</span> — append to tape without
                  replying
                </span>
              </label>
            </div>
          </div>

          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={createMutation.isPending}>
            {createMutation.isPending ? 'Creating…' : 'Create'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
