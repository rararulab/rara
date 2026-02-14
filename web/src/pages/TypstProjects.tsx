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
import { useNavigate } from "react-router";
import { api } from "@/api/client";
import type { TypstProject } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { FileType, Plus, Trash2 } from "lucide-react";

interface ProjectForm {
  name: string;
  description: string;
  main_file: string;
}

const emptyForm: ProjectForm = {
  name: "",
  description: "",
  main_file: "main.typ",
};

export default function TypstProjects() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [form, setForm] = useState<ProjectForm>(emptyForm);
  const [deleteTarget, setDeleteTarget] = useState<TypstProject | null>(null);

  const { data: projects, isLoading } = useQuery({
    queryKey: ["typst-projects"],
    queryFn: () => api.get<TypstProject[]>("/api/v1/typst/projects"),
  });

  const createMutation = useMutation({
    mutationFn: (data: ProjectForm) =>
      api.post<TypstProject>("/api/v1/typst/projects", {
        name: data.name,
        description: data.description || null,
        main_file: data.main_file || "main.typ",
      }),
    onSuccess: (project) => {
      queryClient.invalidateQueries({ queryKey: ["typst-projects"] });
      closeDialog();
      navigate(`/typst/${project.id}`);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.del(`/api/v1/typst/projects/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["typst-projects"] });
      setDeleteTarget(null);
    },
  });

  function closeDialog() {
    setDialogOpen(false);
    setForm(emptyForm);
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!form.name.trim()) return;
    createMutation.mutate(form);
  }

  function formatDate(dateStr: string) {
    return new Date(dateStr).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Typst Projects</h1>
          <p className="text-muted-foreground mt-1">
            Create and manage Typst document projects.
          </p>
        </div>
        <Button onClick={() => setDialogOpen(true)}>
          <Plus className="h-4 w-4 mr-1" />
          New Project
        </Button>
      </div>

      {isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={i} className="h-14 w-full" />
          ))}
        </div>
      ) : !projects || projects.length === 0 ? (
        <div className="rounded-lg border border-dashed p-8 text-center">
          <FileType className="mx-auto h-12 w-12 text-muted-foreground/30 mb-3" />
          <p className="text-muted-foreground">
            No projects yet. Create your first Typst project to get started.
          </p>
        </div>
      ) : (
        <div className="rounded-lg border">
          <table className="w-full">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Name
                </th>
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Description
                </th>
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Main File
                </th>
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Last Updated
                </th>
                <th className="px-4 py-3 text-right text-sm font-medium">
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {projects.map((project) => (
                <tr key={project.id} className="border-b last:border-b-0">
                  <td className="px-4 py-3 text-sm font-medium">
                    <button
                      type="button"
                      className="text-left hover:underline cursor-pointer"
                      onClick={() => navigate(`/typst/${project.id}`)}
                    >
                      {project.name}
                    </button>
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground">
                    {project.description ?? "-"}
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground font-mono">
                    {project.main_file}
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground">
                    {formatDate(project.updated_at)}
                  </td>
                  <td className="px-4 py-3 text-right">
                    <div className="flex items-center justify-end gap-2">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => navigate(`/typst/${project.id}`)}
                      >
                        Open
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="text-destructive"
                        onClick={() => setDeleteTarget(project)}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Create Project Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New Typst Project</DialogTitle>
            <DialogDescription>
              Create a new Typst project. A main file will be created
              automatically.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name">Project Name *</Label>
              <Input
                id="name"
                value={form.name}
                onChange={(e) =>
                  setForm((f) => ({ ...f, name: e.target.value }))
                }
                placeholder="e.g. My Resume"
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                value={form.description}
                onChange={(e) =>
                  setForm((f) => ({ ...f, description: e.target.value }))
                }
                placeholder="Optional project description..."
                rows={3}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="main_file">Main File</Label>
              <Input
                id="main_file"
                value={form.main_file}
                onChange={(e) =>
                  setForm((f) => ({ ...f, main_file: e.target.value }))
                }
                placeholder="main.typ"
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={closeDialog}>
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={createMutation.isPending || !form.name.trim()}
              >
                {createMutation.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Project</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{deleteTarget?.name}&quot;?
              This will delete all files and render history. This action cannot
              be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={deleteMutation.isPending}
              onClick={() => {
                if (deleteTarget) deleteMutation.mutate(deleteTarget.id);
              }}
            >
              {deleteMutation.isPending ? "Deleting..." : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
