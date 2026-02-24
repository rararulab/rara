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
import type { ResumeProject, SshKeyResponse } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { GitBranch, Copy, RefreshCw, Trash2, Check } from "lucide-react";

export default function Resumes() {
  const queryClient = useQueryClient();
  const [copied, setCopied] = useState(false);
  const [name, setName] = useState("");
  const [gitUrl, setGitUrl] = useState("");

  const { data: sshKey, isLoading: sshLoading } = useQuery({
    queryKey: ["ssh-key"],
    queryFn: () => api.get<SshKeyResponse>("/api/v1/auth/ssh-key"),
  });

  const { data: project, isLoading: projectLoading } = useQuery({
    queryKey: ["resume-project"],
    queryFn: () => api.get<ResumeProject | null>("/api/v1/resume-project"),
  });

  const setupMutation = useMutation({
    mutationFn: (data: { name: string; git_url: string }) =>
      api.post<ResumeProject>("/api/v1/resume-project", data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["resume-project"] });
      setName("");
      setGitUrl("");
    },
  });

  const syncMutation = useMutation({
    mutationFn: () => api.post<ResumeProject>("/api/v1/resume-project/sync"),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["resume-project"] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => api.del("/api/v1/resume-project"),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["resume-project"] });
    },
  });

  const copyKey = async () => {
    if (sshKey?.public_key) {
      await navigator.clipboard.writeText(sshKey.public_key);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const isLoading = sshLoading || projectLoading;

  if (isLoading) {
    return (
      <div className="space-y-4 p-6">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-6 p-6">
      <h2 className="text-2xl font-bold">Resume Project</h2>

      {/* SSH Key Section */}
      <Card>
        <CardHeader>
          <CardTitle className="text-lg">SSH Key</CardTitle>
          <CardDescription>
            Add this public key to your GitHub account (Settings → SSH Keys) to allow cloning.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-2">
            <code className="flex-1 rounded bg-muted p-3 text-xs font-mono break-all">
              {sshKey?.public_key || "Loading..."}
            </code>
            <Button variant="outline" size="icon" onClick={copyKey}>
              {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Project Config */}
      {project ? (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle className="text-lg flex items-center gap-2">
                <GitBranch className="h-5 w-5" />
                {project.name}
              </CardTitle>
              <Badge variant="secondary">Connected</Badge>
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-2 text-sm">
              <div>
                <span className="text-muted-foreground">Repository:</span>{" "}
                <code className="text-xs">{project.git_url}</code>
              </div>
              <div>
                <span className="text-muted-foreground">Local Path:</span>{" "}
                <code className="text-xs">{project.local_path}</code>
              </div>
              <div>
                <span className="text-muted-foreground">Last Synced:</span>{" "}
                {project.last_synced_at
                  ? new Date(project.last_synced_at).toLocaleString()
                  : "Never"}
              </div>
            </div>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => syncMutation.mutate()}
                disabled={syncMutation.isPending}
              >
                <RefreshCw className={`h-4 w-4 mr-1 ${syncMutation.isPending ? "animate-spin" : ""}`} />
                Sync
              </Button>
              <Button
                variant="destructive"
                size="sm"
                onClick={() => {
                  if (confirm("Remove this resume project? Local files will be deleted.")) {
                    deleteMutation.mutate();
                  }
                }}
                disabled={deleteMutation.isPending}
              >
                <Trash2 className="h-4 w-4 mr-1" />
                Remove
              </Button>
            </div>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle className="text-lg">Setup Resume Project</CardTitle>
            <CardDescription>
              Connect your GitHub resume repository (Typst project).
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name">Project Name</Label>
              <Input
                id="name"
                placeholder="My Resume"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="git-url">Git SSH URL</Label>
              <Input
                id="git-url"
                placeholder="git@github.com:user/resume.git"
                value={gitUrl}
                onChange={(e) => setGitUrl(e.target.value)}
              />
            </div>
            <Button
              onClick={() => setupMutation.mutate({ name, git_url: gitUrl })}
              disabled={!name || !gitUrl || setupMutation.isPending}
            >
              <GitBranch className="h-4 w-4 mr-1" />
              {setupMutation.isPending ? "Cloning..." : "Clone & Setup"}
            </Button>
            {setupMutation.isError && (
              <p className="text-sm text-destructive">
                {(setupMutation.error as Error).message}
              </p>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
