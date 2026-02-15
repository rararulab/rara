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
import { FileType, FolderOpen, GitBranch, Plus, RefreshCw, Trash2 } from "lucide-react";

interface RegisterForm {
  name: string;
  local_path: string;
  main_file: string;
}

const emptyForm: RegisterForm = {
  name: "",
  local_path: "",
  main_file: "",
};

interface GitImportForm {
  url: string;
  name: string;
  target_dir: string;
}

export default function TypstProjects() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [form, setForm] = useState<RegisterForm>(emptyForm);
  const [deleteTarget, setDeleteTarget] = useState<TypstProject | null>(null);
  const [gitDialogOpen, setGitDialogOpen] = useState(false);
  const [gitForm, setGitForm] = useState<GitImportForm>({
    url: "",
    name: "",
    target_dir: "",
  });

  const { data: projects, isLoading } = useQuery({
    queryKey: ["typst-projects"],
    queryFn: () => api.get<TypstProject[]>("/api/v1/typst/projects"),
  });

  const registerMutation = useMutation({
    mutationFn: (data: RegisterForm) =>
      api.post<TypstProject>("/api/v1/typst/projects", {
        name: data.name,
        local_path: data.local_path,
        main_file: data.main_file || undefined,
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

  const gitImportMutation = useMutation({
    mutationFn: (data: GitImportForm) =>
      api.importTypstFromGit({
        url: data.url,
        name: data.name || undefined,
        target_dir: data.target_dir,
      }),
    onSuccess: (project) => {
      queryClient.invalidateQueries({ queryKey: ["typst-projects"] });
      setGitDialogOpen(false);
      setGitForm({ url: "", name: "", target_dir: "" });
      navigate(`/typst/${project.id}`);
    },
  });

  const syncGitMutation = useMutation({
    mutationFn: (id: string) => api.syncTypstGit(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["typst-projects"] });
    },
  });

  function closeDialog() {
    setDialogOpen(false);
    setForm(emptyForm);
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!form.name.trim() || !form.local_path.trim()) return;
    registerMutation.mutate(form);
  }

  function handleGitImport(e: React.FormEvent) {
    e.preventDefault();
    if (!gitForm.url.trim() || !gitForm.target_dir.trim()) return;
    gitImportMutation.mutate(gitForm);
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
            Register local Typst project directories.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" onClick={() => setGitDialogOpen(true)}>
            <GitBranch className="h-4 w-4 mr-1" />
            Import from Git
          </Button>
          <Button onClick={() => setDialogOpen(true)}>
            <Plus className="h-4 w-4 mr-1" />
            Add Project
          </Button>
        </div>
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
            No projects yet. Add a local Typst project directory to get started.
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
                  Local Path
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
                    <div className="flex items-center gap-1.5">
                      {project.git_url ? (
                        <span title={project.git_url}>
                          <GitBranch className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                        </span>
                      ) : (
                        <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                      )}
                      <button
                        type="button"
                        className="text-left hover:underline cursor-pointer"
                        onClick={() => navigate(`/typst/${project.id}`)}
                      >
                        {project.name}
                      </button>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground font-mono text-xs max-w-xs truncate">
                    {project.local_path}
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground font-mono">
                    {project.main_file}
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground">
                    {formatDate(project.updated_at)}
                  </td>
                  <td className="px-4 py-3 text-right">
                    <div className="flex items-center justify-end gap-2">
                      {project.git_url && (
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={syncGitMutation.isPending}
                          onClick={() => syncGitMutation.mutate(project.id)}
                          title={
                            project.git_last_synced_at
                              ? `Last synced: ${formatDate(project.git_last_synced_at)}`
                              : "Never synced"
                          }
                        >
                          <RefreshCw
                            className={`h-3.5 w-3.5 mr-1 ${syncGitMutation.isPending ? "animate-spin" : ""}`}
                          />
                          Sync
                        </Button>
                      )}
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

      {/* Add Project Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add Typst Project</DialogTitle>
            <DialogDescription>
              Register an existing local directory as a Typst project. The
              directory must contain at least one .typ file.
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
              <Label htmlFor="local_path">Local Directory Path *</Label>
              <Input
                id="local_path"
                value={form.local_path}
                onChange={(e) =>
                  setForm((f) => ({ ...f, local_path: e.target.value }))
                }
                placeholder="/Users/you/Documents/my-resume"
                required
              />
              <p className="text-xs text-muted-foreground">
                Absolute path to the directory containing your .typ files.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="main_file">Main File (optional)</Label>
              <Input
                id="main_file"
                value={form.main_file}
                onChange={(e) =>
                  setForm((f) => ({ ...f, main_file: e.target.value }))
                }
                placeholder="Auto-detected (defaults to main.typ)"
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={closeDialog}>
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={
                  registerMutation.isPending ||
                  !form.name.trim() ||
                  !form.local_path.trim()
                }
              >
                {registerMutation.isPending ? "Adding..." : "Add Project"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Git Import Dialog */}
      <Dialog open={gitDialogOpen} onOpenChange={setGitDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Import from Git</DialogTitle>
            <DialogDescription>
              Clone a Git repository into a local directory and register it as a
              Typst project.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleGitImport} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="git-url">Repository URL *</Label>
              <Input
                id="git-url"
                value={gitForm.url}
                onChange={(e) =>
                  setGitForm((f) => ({ ...f, url: e.target.value }))
                }
                placeholder="https://github.com/user/repo.git"
                required
              />
              <p className="text-xs text-muted-foreground">
                Only HTTPS URLs are supported.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="git-target-dir">Target Directory *</Label>
              <Input
                id="git-target-dir"
                value={gitForm.target_dir}
                onChange={(e) =>
                  setGitForm((f) => ({ ...f, target_dir: e.target.value }))
                }
                placeholder="/Users/you/Documents/imported-project"
                required
              />
              <p className="text-xs text-muted-foreground">
                Local directory to clone the repository into.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="git-name">Project Name (optional)</Label>
              <Input
                id="git-name"
                value={gitForm.name}
                onChange={(e) =>
                  setGitForm((f) => ({ ...f, name: e.target.value }))
                }
                placeholder="Auto-detected from repository name"
              />
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setGitDialogOpen(false);
                  setGitForm({ url: "", name: "", target_dir: "" });
                }}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={
                  gitImportMutation.isPending ||
                  !gitForm.url.trim() ||
                  !gitForm.target_dir.trim()
                }
              >
                {gitImportMutation.isPending ? "Importing..." : "Import"}
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
              This will remove the project registration and render history. Your
              local files will NOT be deleted.
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
