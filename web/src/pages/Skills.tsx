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
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { listSkills, getSkill, createSkill, deleteSkill } from '@/api/skills';
import type { SkillSummary, CreateSkillRequest } from '@/api/types';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { Separator } from '@/components/ui/separator';
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
import { Plus, Trash2, Code, ExternalLink, CheckCircle, XCircle } from 'lucide-react';

// ---------------------------------------------------------------------------
// Skill Form Dialog (create only)
// ---------------------------------------------------------------------------

interface SkillFormData {
  name: string;
  description: string;
  allowed_tools: string;
  prompt: string;
}

const EMPTY_FORM: SkillFormData = {
  name: '',
  description: '',
  allowed_tools: '',
  prompt: '',
};

function SkillFormDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<SkillFormData>({ ...EMPTY_FORM });

  const mutation = useMutation({
    mutationFn: (data: SkillFormData): Promise<unknown> => {
      const req: CreateSkillRequest = {
        name: data.name,
        description: data.description,
        allowed_tools: data.allowed_tools
          .split(',')
          .map((t) => t.trim())
          .filter(Boolean),
        prompt: data.prompt,
      };
      return createSkill(req);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['skills'] });
      onOpenChange(false);
    },
  });

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    mutation.mutate(form);
  }

  function updateField<K extends keyof SkillFormData>(key: K, value: SkillFormData[K]) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <DialogTitle>Create Skill</DialogTitle>
          <DialogDescription>
            Define a new skill with allowed tools and a prompt body.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="name">Name *</Label>
            <Input
              id="name"
              value={form.name}
              onChange={(e) => updateField('name', e.target.value)}
              placeholder="e.g. commit"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="description">Description *</Label>
            <Input
              id="description"
              value={form.description}
              onChange={(e) => updateField('description', e.target.value)}
              placeholder="e.g. Create git commits"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="allowed_tools">Allowed Tools *</Label>
            <Input
              id="allowed_tools"
              value={form.allowed_tools}
              onChange={(e) => updateField('allowed_tools', e.target.value)}
              placeholder="e.g. Read, Write, Bash (comma-separated)"
              required
            />
            <p className="text-xs text-muted-foreground">Comma-separated list of tool names.</p>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="prompt">Body *</Label>
            <Textarea
              id="prompt"
              value={form.prompt}
              onChange={(e) => updateField('prompt', e.target.value)}
              placeholder="Markdown content for this skill..."
              rows={6}
              required
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? 'Creating...' : 'Create'}
            </Button>
          </DialogFooter>
          {mutation.isError && (
            <p className="text-sm text-destructive">Error: {(mutation.error as Error).message}</p>
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
  skill,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  skill: SkillSummary;
}) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => deleteSkill(skill.name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['skills'] });
      onOpenChange(false);
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <DialogTitle>Delete Skill</DialogTitle>
          <DialogDescription>
            Are you sure you want to delete the skill <strong>{skill.name}</strong>? This action
            cannot be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? 'Deleting...' : 'Delete'}
          </Button>
        </DialogFooter>
        {mutation.isError && (
          <p className="text-sm text-destructive mt-2">
            Error: {(mutation.error as Error).message}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Skill Detail Dialog
// ---------------------------------------------------------------------------

function SkillDetailDialog({
  open,
  onOpenChange,
  skillName,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  skillName: string;
}) {
  const { data, isLoading, isError } = useQuery({
    queryKey: ['skills', skillName],
    queryFn: () => getSkill(skillName),
    enabled: open,
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[700px] max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Code className="h-5 w-5" />
            {skillName}
          </DialogTitle>
          {data && <DialogDescription>{data.description}</DialogDescription>}
        </DialogHeader>
        <div className="flex-1 overflow-y-auto">
          {isLoading && (
            <div className="space-y-2 py-4">
              <Skeleton className="h-4 w-3/4" />
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-5/6" />
              <Skeleton className="h-4 w-2/3" />
            </div>
          )}
          {isError && (
            <p className="text-sm text-destructive py-4">Failed to load skill details.</p>
          )}
          {data && (
            <div className="space-y-4 py-2">
              {/* Metadata row */}
              <div className="flex flex-wrap gap-2">
                {data.source && <Badge variant={sourceVariant(data.source)}>{data.source}</Badge>}
                {data.eligible ? (
                  <Badge variant="outline" className="text-green-600 border-green-600/30">
                    <CheckCircle className="h-3 w-3 mr-1" />
                    Eligible
                  </Badge>
                ) : (
                  <Badge variant="outline" className="text-destructive border-destructive/30">
                    <XCircle className="h-3 w-3 mr-1" />
                    Missing deps
                  </Badge>
                )}
                {data.license && <Badge variant="outline">{data.license}</Badge>}
              </div>

              {/* Allowed tools */}
              {data.allowed_tools.length > 0 && (
                <div>
                  <p className="text-xs font-medium text-muted-foreground mb-1.5">Allowed Tools</p>
                  <div className="flex flex-wrap gap-1.5">
                    {data.allowed_tools.map((tool) => (
                      <Badge key={tool} variant="secondary" className="text-xs">
                        {tool}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}

              {/* Homepage */}
              {data.homepage && (
                <a
                  href={data.homepage}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
                >
                  <ExternalLink className="h-3 w-3" />
                  {data.homepage}
                </a>
              )}

              <Separator />

              {/* Body content */}
              <div className="prose prose-sm max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{data.body}</ReactMarkdown>
              </div>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Loading Skeleton
// ---------------------------------------------------------------------------

function SkillsSkeleton() {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
      {Array.from({ length: 6 }).map((_, i) => (
        <Skeleton key={i} className="h-48 rounded-lg" />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Page
// ---------------------------------------------------------------------------

export default function Skills() {
  const [createOpen, setCreateOpen] = useState(false);
  const [detailSkillName, setDetailSkillName] = useState<string | null>(null);
  const [deleteSkillItem, setDeleteSkillItem] = useState<SkillSummary | null>(null);

  const {
    data: skills,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ['skills'],
    queryFn: listSkills,
  });

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Skills</h1>
          <p className="text-muted-foreground mt-1">Manage agent skills and allowed tools.</p>
        </div>
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="h-4 w-4" />
          Create Skill
        </Button>
      </div>

      <Separator />

      {/* Content */}
      {isLoading && <SkillsSkeleton />}

      {isError && (
        <div className="rounded-lg border border-destructive/50 p-4 text-sm text-destructive">
          Failed to load skills: {(error as Error).message}
        </div>
      )}

      {skills && skills.length === 0 && (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
          <p className="text-lg font-medium">No skills defined</p>
          <p className="text-sm text-muted-foreground mt-1">
            Get started by creating your first skill.
          </p>
          <Button className="mt-4" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            Create Skill
          </Button>
        </div>
      )}

      {skills && skills.length > 0 && (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {skills.map((skill) => (
            <SkillCard
              key={skill.name}
              skill={skill}
              onClick={() => setDetailSkillName(skill.name)}
              onDelete={() => setDeleteSkillItem(skill)}
            />
          ))}
        </div>
      )}

      {/* Dialogs */}
      {detailSkillName && (
        <SkillDetailDialog
          open={true}
          onOpenChange={(open) => {
            if (!open) setDetailSkillName(null);
          }}
          skillName={detailSkillName}
        />
      )}

      {createOpen && (
        <SkillFormDialog
          open={createOpen}
          onOpenChange={(open) => {
            setCreateOpen(open);
          }}
        />
      )}

      {deleteSkillItem && (
        <DeleteDialog
          key={deleteSkillItem.name}
          open={true}
          onOpenChange={(open) => {
            if (!open) setDeleteSkillItem(null);
          }}
          skill={deleteSkillItem}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Source badge variant helper
// ---------------------------------------------------------------------------

function sourceVariant(source: string | null): 'default' | 'secondary' | 'outline' | 'destructive' {
  switch (source) {
    case 'project':
      return 'default';
    case 'personal':
      return 'secondary';
    case 'plugin':
      return 'outline';
    case 'registry':
      return 'outline';
    default:
      return 'secondary';
  }
}

// ---------------------------------------------------------------------------
// Skill Card Component
// ---------------------------------------------------------------------------

function SkillCard({
  skill,
  onClick,
  onDelete,
}: {
  skill: SkillSummary;
  onClick: () => void;
  onDelete: () => void;
}) {
  return (
    <div
      className="rounded-lg border bg-card p-4 space-y-3 hover:bg-accent/5 transition-colors cursor-pointer"
      onClick={onClick}
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <Code className="h-5 w-5 text-muted-foreground shrink-0" />
          <h3 className="font-semibold text-lg truncate">{skill.name}</h3>
        </div>
        {skill.source && (
          <Badge variant={sourceVariant(skill.source)} className="shrink-0">
            {skill.source}
          </Badge>
        )}
      </div>

      {/* Description */}
      <p className="text-sm text-muted-foreground">{skill.description}</p>

      {/* Allowed Tools */}
      {skill.allowed_tools.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {skill.allowed_tools.map((tool) => (
            <Badge key={tool} variant="outline" className="text-xs">
              {tool}
            </Badge>
          ))}
        </div>
      )}

      {/* Metadata: license & homepage */}
      {(skill.license || skill.homepage) && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
          {skill.license && <span>License: {skill.license}</span>}
          {skill.homepage && (
            <a
              href={skill.homepage}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 hover:text-foreground transition-colors"
              onClick={(e) => e.stopPropagation()}
            >
              Homepage
              <ExternalLink className="h-3 w-3" />
            </a>
          )}
        </div>
      )}

      <Separator />

      {/* Footer: eligible status + delete action */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          {skill.eligible ? (
            <>
              <CheckCircle className="h-4 w-4 text-green-600" />
              <span className="text-xs text-green-600">Eligible</span>
            </>
          ) : (
            <>
              <XCircle className="h-4 w-4 text-destructive" />
              <span className="text-xs text-destructive">Missing deps</span>
            </>
          )}
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
          title="Delete skill"
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
