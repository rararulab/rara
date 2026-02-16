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
import { listSkills, createSkill, updateSkill, deleteSkill } from "@/api/skills";
import type { Skill } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Plus, Pencil, Trash2, Code } from "lucide-react";

// ---------------------------------------------------------------------------
// Skill Form Dialog
// ---------------------------------------------------------------------------

interface SkillFormData {
  name: string;
  description: string;
  tools: string;
  trigger: string;
  prompt: string;
}

const EMPTY_FORM: SkillFormData = {
  name: "",
  description: "",
  tools: "",
  trigger: "",
  prompt: "",
};

function SkillFormDialog({
  open,
  onOpenChange,
  initialData,
  mode,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialData?: Skill;
  mode: "create" | "edit";
}) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<SkillFormData>(() =>
    initialData
      ? {
          name: initialData.name,
          description: initialData.description,
          tools: initialData.tools.join(", "),
          trigger: initialData.trigger ?? "",
          prompt: initialData.prompt ?? "",
        }
      : { ...EMPTY_FORM }
  );

  const createMutation = useMutation({
    mutationFn: (data: SkillFormData) =>
      createSkill({
        name: data.name,
        description: data.description,
        tools: data.tools.split(",").map((t) => t.trim()).filter(Boolean),
        trigger: data.trigger || null,
        prompt: data.prompt || undefined,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
      onOpenChange(false);
    },
  });

  const updateMutation = useMutation({
    mutationFn: (data: SkillFormData) =>
      updateSkill(initialData!.name, {
        description: data.description,
        tools: data.tools.split(",").map((t) => t.trim()).filter(Boolean),
        trigger: data.trigger || null,
        prompt: data.prompt || undefined,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
      onOpenChange(false);
    },
  });

  const mutation = mode === "create" ? createMutation : updateMutation;

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    mutation.mutate(form);
  }

  function updateField<K extends keyof SkillFormData>(
    key: K,
    value: SkillFormData[K]
  ) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <DialogTitle>
            {mode === "create" ? "Create Skill" : "Edit Skill"}
          </DialogTitle>
          <DialogDescription>
            {mode === "create"
              ? "Define a new skill with tools and triggers."
              : "Update the skill configuration."}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="name">Name *</Label>
            <Input
              id="name"
              value={form.name}
              onChange={(e) => updateField("name", e.target.value)}
              placeholder="e.g. commit"
              required
              disabled={mode === "edit"}
            />
            {mode === "edit" && (
              <p className="text-xs text-muted-foreground">
                Skill name cannot be changed.
              </p>
            )}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="description">Description *</Label>
            <Input
              id="description"
              value={form.description}
              onChange={(e) => updateField("description", e.target.value)}
              placeholder="e.g. Create git commits"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="tools">Tools *</Label>
            <Input
              id="tools"
              value={form.tools}
              onChange={(e) => updateField("tools", e.target.value)}
              placeholder="e.g. Read, Write, Bash (comma-separated)"
              required
            />
            <p className="text-xs text-muted-foreground">
              Comma-separated list of tool names.
            </p>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="trigger">Trigger Pattern</Label>
            <Input
              id="trigger"
              value={form.trigger}
              onChange={(e) => updateField("trigger", e.target.value)}
              placeholder="e.g. /commit"
            />
            <p className="text-xs text-muted-foreground">
              Optional regex pattern to auto-trigger this skill.
            </p>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="prompt">Prompt</Label>
            <Textarea
              id="prompt"
              value={form.prompt}
              onChange={(e) => updateField("prompt", e.target.value)}
              placeholder="System prompt for this skill..."
              rows={6}
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
              {mutation.isPending
                ? "Saving..."
                : mode === "create"
                  ? "Create"
                  : "Save"}
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
  skill,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  skill: Skill;
}) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => deleteSkill(skill.name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
      onOpenChange(false);
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <DialogTitle>Delete Skill</DialogTitle>
          <DialogDescription>
            Are you sure you want to delete the skill{" "}
            <strong>{skill.name}</strong>? This action cannot be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? "Deleting..." : "Delete"}
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
  const [editSkill, setEditSkill] = useState<Skill | null>(null);
  const [deleteSkillItem, setDeleteSkillItem] = useState<Skill | null>(null);

  const queryClient = useQueryClient();

  const {
    data: skills,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ["skills"],
    queryFn: listSkills,
  });

  const toggleEnabledMutation = useMutation({
    mutationFn: ({ name, enabled }: { name: string; enabled: boolean }) =>
      updateSkill(name, { enabled }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  function handleToggleEnabled(skill: Skill) {
    toggleEnabledMutation.mutate({ name: skill.name, enabled: !skill.enabled });
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Skills</h1>
          <p className="text-muted-foreground mt-1">
            Manage agent skills, tools, and triggers.
          </p>
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
              onEdit={() => setEditSkill(skill)}
              onDelete={() => setDeleteSkillItem(skill)}
              onToggleEnabled={() => handleToggleEnabled(skill)}
            />
          ))}
        </div>
      )}

      {/* Dialogs */}
      {createOpen && (
        <SkillFormDialog
          open={createOpen}
          onOpenChange={(open) => {
            setCreateOpen(open);
          }}
          mode="create"
        />
      )}

      {editSkill && (
        <SkillFormDialog
          key={editSkill.name}
          open={true}
          onOpenChange={(open) => {
            if (!open) setEditSkill(null);
          }}
          initialData={editSkill}
          mode="edit"
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
// Skill Card Component
// ---------------------------------------------------------------------------

function SkillCard({
  skill,
  onEdit,
  onDelete,
  onToggleEnabled,
}: {
  skill: Skill;
  onEdit: () => void;
  onDelete: () => void;
  onToggleEnabled: () => void;
}) {
  return (
    <div className="rounded-lg border bg-card p-4 space-y-3 hover:bg-accent/5 transition-colors">
      {/* Header */}
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2">
          <Code className="h-5 w-5 text-muted-foreground shrink-0" />
          <h3 className="font-semibold text-lg">{skill.name}</h3>
        </div>
        <Badge variant={skill.enabled ? "default" : "secondary"}>
          {skill.enabled ? "Enabled" : "Disabled"}
        </Badge>
      </div>

      {/* Description */}
      <p className="text-sm text-muted-foreground">{skill.description}</p>

      {/* Tools */}
      <div className="flex flex-wrap gap-1.5">
        {skill.tools.map((tool) => (
          <Badge key={tool} variant="outline" className="text-xs">
            {tool}
          </Badge>
        ))}
      </div>

      {/* Trigger */}
      {skill.trigger && (
        <div className="pt-1">
          <p className="text-xs text-muted-foreground mb-1">Trigger:</p>
          <code className="text-xs bg-muted px-2 py-0.5 rounded">
            {skill.trigger}
          </code>
        </div>
      )}

      <Separator />

      {/* Actions */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Switch
            checked={skill.enabled}
            onCheckedChange={onToggleEnabled}
            aria-label="Toggle enabled"
          />
          <span className="text-xs text-muted-foreground">
            {skill.enabled ? "Active" : "Inactive"}
          </span>
        </div>
        <div className="flex gap-1">
          <Button
            variant="ghost"
            size="sm"
            onClick={onEdit}
            title="Edit skill"
          >
            <Pencil className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            title="Delete skill"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  );
}
