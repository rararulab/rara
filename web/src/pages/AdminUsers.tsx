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

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { adminApi } from "@/api/client";
import type { InviteCode, PlatformInfo, UserInfo } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Check,
  Clipboard,
  Plus,
  ShieldCheck,
  UserX,
  Users,
} from "lucide-react";

type AdminUser = UserInfo & { platforms?: PlatformInfo[] };

function roleBadge(role: string) {
  switch (role.toLowerCase()) {
    case "root":
      return <Badge className="bg-red-600 text-white hover:bg-red-700">{role}</Badge>;
    case "admin":
      return <Badge className="bg-blue-600 text-white hover:bg-blue-700">{role}</Badge>;
    default:
      return <Badge variant="secondary">{role}</Badge>;
  }
}

function enabledBadge(enabled: boolean) {
  return enabled ? (
    <Badge className="bg-green-600 text-white hover:bg-green-700">Enabled</Badge>
  ) : (
    <Badge variant="destructive">Disabled</Badge>
  );
}

function inviteStatus(code: InviteCode): { label: string; variant: "default" | "secondary" | "destructive" } {
  if (code.used_by) return { label: "Used", variant: "secondary" };
  if (new Date(code.expires_at) < new Date()) return { label: "Expired", variant: "destructive" };
  return { label: "Available", variant: "default" };
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <Button
      variant="ghost"
      size="icon"
      className="h-7 w-7 shrink-0"
      onClick={handleCopy}
      title="Copy to clipboard"
    >
      {copied ? <Check className="h-3.5 w-3.5 text-green-600" /> : <Clipboard className="h-3.5 w-3.5" />}
    </Button>
  );
}

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function AdminUsers() {
  const queryClient = useQueryClient();
  const [disableTarget, setDisableTarget] = useState<AdminUser | null>(null);

  const usersQuery = useQuery({
    queryKey: ["admin", "users"],
    queryFn: adminApi.listUsers,
  });

  const inviteCodesQuery = useQuery({
    queryKey: ["admin", "invite-codes"],
    queryFn: adminApi.listInviteCodes,
  });

  const disableMutation = useMutation({
    mutationFn: (id: string) => adminApi.disableUser(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
      setDisableTarget(null);
    },
  });

  const createInviteMutation = useMutation({
    mutationFn: () => adminApi.createInviteCode(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["admin", "invite-codes"] });
    },
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <ShieldCheck className="h-6 w-6 text-muted-foreground" />
        <div>
          <h1 className="text-2xl font-bold tracking-tight">User Management</h1>
          <p className="text-sm text-muted-foreground">
            Manage users and invite codes
          </p>
        </div>
      </div>

      <Tabs defaultValue="users">
        <TabsList>
          <TabsTrigger value="users" className="gap-1.5">
            <Users className="h-3.5 w-3.5" />
            Users
          </TabsTrigger>
          <TabsTrigger value="invites" className="gap-1.5">
            <Plus className="h-3.5 w-3.5" />
            Invite Codes
          </TabsTrigger>
        </TabsList>

        {/* ── Users Tab ── */}
        <TabsContent value="users">
          {usersQuery.isLoading ? (
            <div className="space-y-3 pt-4">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </div>
          ) : usersQuery.isError ? (
            <p className="pt-4 text-sm text-destructive">
              Failed to load users. Please try again.
            </p>
          ) : (
            <div className="rounded-lg border mt-4">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>Role</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Platforms</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {usersQuery.data?.map((user) => (
                    <TableRow key={user.id}>
                      <TableCell className="font-medium">{user.name}</TableCell>
                      <TableCell>{roleBadge(user.role)}</TableCell>
                      <TableCell>{enabledBadge(user.enabled)}</TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1">
                          {user.platforms && user.platforms.length > 0 ? (
                            user.platforms.map((p) => (
                              <Badge key={p.platform} variant="outline" className="text-[10px]">
                                {p.platform}
                                {p.display_name ? `: ${p.display_name}` : ""}
                              </Badge>
                            ))
                          ) : (
                            <span className="text-xs text-muted-foreground">None</span>
                          )}
                        </div>
                      </TableCell>
                      <TableCell className="text-right">
                        {user.enabled && user.role !== "root" && (
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-destructive"
                            onClick={() => setDisableTarget(user)}
                          >
                            <UserX className="h-3.5 w-3.5" />
                            Disable
                          </Button>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                  {usersQuery.data?.length === 0 && (
                    <TableRow>
                      <TableCell colSpan={5} className="text-center text-muted-foreground">
                        No users found.
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </div>
          )}
        </TabsContent>

        {/* ── Invite Codes Tab ── */}
        <TabsContent value="invites">
          <div className="flex items-center justify-between pt-4 pb-3">
            <p className="text-sm text-muted-foreground">
              {inviteCodesQuery.data?.length ?? 0} invite code(s)
            </p>
            <Button
              size="sm"
              onClick={() => createInviteMutation.mutate()}
              disabled={createInviteMutation.isPending}
            >
              <Plus className="mr-1.5 h-3.5 w-3.5" />
              {createInviteMutation.isPending ? "Generating..." : "Generate Invite Code"}
            </Button>
          </div>

          {inviteCodesQuery.isLoading ? (
            <div className="space-y-3">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </div>
          ) : inviteCodesQuery.isError ? (
            <p className="text-sm text-destructive">
              Failed to load invite codes. Please try again.
            </p>
          ) : (
            <div className="rounded-lg border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Code</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Created At</TableHead>
                    <TableHead>Expires At</TableHead>
                    <TableHead>Used By</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {inviteCodesQuery.data?.map((code) => {
                    const status = inviteStatus(code);
                    return (
                      <TableRow key={code.id}>
                        <TableCell>
                          <div className="flex items-center gap-1">
                            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
                              {code.code}
                            </code>
                            <CopyButton text={code.code} />
                          </div>
                        </TableCell>
                        <TableCell>
                          <Badge variant={status.variant}>{status.label}</Badge>
                        </TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {formatDate(code.created_at)}
                        </TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {formatDate(code.expires_at)}
                        </TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {code.used_by ?? "--"}
                        </TableCell>
                      </TableRow>
                    );
                  })}
                  {inviteCodesQuery.data?.length === 0 && (
                    <TableRow>
                      <TableCell colSpan={5} className="text-center text-muted-foreground">
                        No invite codes yet. Click &quot;Generate Invite Code&quot; to create one.
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </div>
          )}
        </TabsContent>
      </Tabs>

      {/* Disable User Confirmation Dialog */}
      <Dialog open={!!disableTarget} onOpenChange={(open) => !open && setDisableTarget(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Disable User</DialogTitle>
            <DialogDescription>
              Are you sure you want to disable <strong>{disableTarget?.name}</strong>?
              They will no longer be able to log in.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDisableTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => disableTarget && disableMutation.mutate(disableTarget.id)}
              disabled={disableMutation.isPending}
            >
              {disableMutation.isPending ? "Disabling..." : "Disable User"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
