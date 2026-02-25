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
import type {
  Application,
  StatusChangeRecord,
  ApplicationStatus,
} from "@/api/types";
import { APPLICATION_STATUSES } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import {
  Card,
  CardContent,
} from "@/components/ui/card";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Plus,
  Pencil,
  Trash2,
  ExternalLink,
  Clock,
  ArrowRight,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

const STATUS_LABELS: Record<ApplicationStatus, string> = {
  draft: "Draft",
  applied: "Applied",
  screening: "Screening",
  interviewing: "Interviewing",
  offer: "Offer",
  accepted: "Accepted",
  rejected: "Rejected",
  withdrawn: "Withdrawn",
};

function statusBadgeClass(status: string): string {
  switch (status) {
    case "draft":
      return "border-transparent bg-secondary text-secondary-foreground";
    case "applied":
      return "border-transparent bg-primary text-primary-foreground";
    case "screening":
      return "border-border/70 bg-background/70 text-foreground";
    case "interviewing":
      return "border-transparent bg-blue-500/90 text-white";
    case "offer":
      return "border-transparent bg-emerald-600 text-white";
    case "accepted":
      return "border-transparent bg-emerald-700 text-white";
    case "rejected":
      return "border-transparent bg-rose-500 text-white";
    case "withdrawn":
      return "border-transparent bg-zinc-400 text-white";
    default:
      return "";
  }
}

function StatusBadge({ status }: { status: string }) {
  return (
    <Badge
      className={statusBadgeClass(status)}
    >
      {STATUS_LABELS[status as ApplicationStatus] ?? status}
    </Badge>
  );
}

function formatDate(dateStr: string | null): string {
  if (!dateStr) return "-";
  return new Date(dateStr).toLocaleDateString();
}

// ---------------------------------------------------------------------------
// Application Form Dialog
// ---------------------------------------------------------------------------

interface ApplicationFormData {
  company_name: string;
  position_title: string;
  job_url: string;
  notes: string;
  status: string;
}

const EMPTY_FORM: ApplicationFormData = {
  company_name: "",
  position_title: "",
  job_url: "",
  notes: "",
  status: "draft",
};

function ApplicationFormDialog({
  open,
  onOpenChange,
  initialData,
  mode,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialData?: Application;
  mode: "create" | "edit";
}) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<ApplicationFormData>(() =>
    initialData
      ? {
          company_name: initialData.company_name,
          position_title: initialData.position_title,
          job_url: initialData.job_url ?? "",
          notes: initialData.notes ?? "",
          status: initialData.status,
        }
      : { ...EMPTY_FORM }
  );

  const createMutation = useMutation({
    mutationFn: (data: ApplicationFormData) =>
      api.post<Application>("/api/v1/applications", {
        company_name: data.company_name,
        position_title: data.position_title,
        job_url: data.job_url || null,
        notes: data.notes || null,
        status: data.status,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["applications"] });
      onOpenChange(false);
    },
  });

  const updateMutation = useMutation({
    mutationFn: (data: ApplicationFormData) =>
      api.put<Application>(`/api/v1/applications/${initialData!.id}`, {
        company_name: data.company_name,
        position_title: data.position_title,
        job_url: data.job_url || null,
        notes: data.notes || null,
        status: data.status,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["applications"] });
      onOpenChange(false);
    },
  });

  const mutation = mode === "create" ? createMutation : updateMutation;

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    mutation.mutate(form);
  }

  function updateField<K extends keyof ApplicationFormData>(
    key: K,
    value: ApplicationFormData[K]
  ) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>
            {mode === "create" ? "New Application" : "Edit Application"}
          </DialogTitle>
          <DialogDescription>
            {mode === "create"
              ? "Add a new job application to track."
              : "Update the application details."}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="company_name">Company Name *</Label>
            <Input
              id="company_name"
              value={form.company_name}
              onChange={(e) => updateField("company_name", e.target.value)}
              placeholder="e.g. Acme Inc."
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="position_title">Position Title *</Label>
            <Input
              id="position_title"
              value={form.position_title}
              onChange={(e) => updateField("position_title", e.target.value)}
              placeholder="e.g. Senior Software Engineer"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="job_url">Job URL</Label>
            <Input
              id="job_url"
              type="url"
              value={form.job_url}
              onChange={(e) => updateField("job_url", e.target.value)}
              placeholder="https://..."
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="status">Status</Label>
            <Select
              value={form.status}
              onValueChange={(v) => updateField("status", v)}
            >
              <SelectTrigger id="status">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {APPLICATION_STATUSES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {STATUS_LABELS[s]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="notes">Notes</Label>
            <Textarea
              id="notes"
              value={form.notes}
              onChange={(e) => updateField("notes", e.target.value)}
              placeholder="Any additional notes..."
              rows={3}
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
// Status Transition Dialog
// ---------------------------------------------------------------------------

function TransitionDialog({
  open,
  onOpenChange,
  application,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  application: Application;
}) {
  const queryClient = useQueryClient();
  const [targetStatus, setTargetStatus] = useState(application.status);
  const [note, setNote] = useState("");

  const mutation = useMutation({
    mutationFn: () =>
      api.post<Application>(
        `/api/v1/applications/${application.id}/transition`,
        {
          status: targetStatus,
          note: note || null,
        }
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["applications"] });
      queryClient.invalidateQueries({
        queryKey: ["application-history", application.id],
      });
      onOpenChange(false);
    },
  });

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    mutation.mutate();
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>Transition Status</DialogTitle>
          <DialogDescription>
            Change the status of{" "}
            <strong>
              {application.position_title} @ {application.company_name}
            </strong>
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-4 py-4">
          <div className="flex items-center gap-2 text-sm">
            <StatusBadge status={application.status} />
            <ArrowRight className="h-4 w-4 text-muted-foreground" />
            <Select value={targetStatus} onValueChange={setTargetStatus}>
              <SelectTrigger className="w-[160px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {APPLICATION_STATUSES.filter(
                  (s) => s !== application.status
                ).map((s) => (
                  <SelectItem key={s} value={s}>
                    {STATUS_LABELS[s]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="transition-note">Note (optional)</Label>
            <Textarea
              id="transition-note"
              value={note}
              onChange={(e) => setNote(e.target.value)}
              placeholder="Reason for transition..."
              rows={2}
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
            <Button
              type="submit"
              disabled={
                mutation.isPending || targetStatus === application.status
              }
            >
              {mutation.isPending ? "Transitioning..." : "Confirm"}
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
  application,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  application: Application;
}) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () =>
      api.del<void>(`/api/v1/applications/${application.id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["applications"] });
      onOpenChange(false);
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <DialogTitle>Delete Application</DialogTitle>
          <DialogDescription>
            Are you sure you want to delete the application for{" "}
            <strong>
              {application.position_title} @ {application.company_name}
            </strong>
            ? This action cannot be undone.
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
// History Panel
// ---------------------------------------------------------------------------

function HistoryPanel({ applicationId }: { applicationId: string }) {
  const { data: history, isLoading } = useQuery({
    queryKey: ["application-history", applicationId],
    queryFn: () =>
      api.get<StatusChangeRecord[]>(
        `/api/v1/applications/${applicationId}/history`
      ),
  });

  if (isLoading) {
    return (
      <div className="space-y-2 p-4">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="h-4 w-40" />
        <Skeleton className="h-4 w-44" />
      </div>
    );
  }

  if (!history || history.length === 0) {
    return (
      <p className="text-sm text-muted-foreground p-4">
        No status changes recorded.
      </p>
    );
  }

  return (
    <div className="space-y-3 p-4">
      <h4 className="text-sm font-semibold">Status History</h4>
      <div className="space-y-2">
        {history.map((record) => (
          <div
            key={record.id}
            className="flex items-start gap-2 text-sm border-l-2 border-border pl-3 py-1"
          >
            <Clock className="h-3.5 w-3.5 mt-0.5 text-muted-foreground shrink-0" />
            <div className="min-w-0">
              <div className="flex items-center gap-1.5 flex-wrap">
                {record.from_status && (
                  <>
                    <StatusBadge status={record.from_status} />
                    <ArrowRight className="h-3 w-3 text-muted-foreground" />
                  </>
                )}
                <StatusBadge status={record.to_status} />
              </div>
              {record.note && (
                <p className="text-muted-foreground mt-0.5">{record.note}</p>
              )}
              <p className="text-xs text-muted-foreground mt-0.5">
                {formatDate(record.changed_at)}
              </p>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Loading Skeleton
// ---------------------------------------------------------------------------

function TableSkeleton() {
  return (
    <div className="app-surface overflow-hidden rounded-2xl border border-border/60 p-4">
      <div className="space-y-3">
      {Array.from({ length: 5 }).map((_, i) => (
        <div key={i} className="flex gap-4">
          <Skeleton className="h-8 flex-1" />
          <Skeleton className="h-8 flex-1" />
          <Skeleton className="h-8 w-24" />
          <Skeleton className="h-8 w-28" />
          <Skeleton className="h-8 w-32" />
        </div>
      ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Page
// ---------------------------------------------------------------------------

export default function Applications() {
  const [createOpen, setCreateOpen] = useState(false);
  const [editApp, setEditApp] = useState<Application | null>(null);
  const [transitionApp, setTransitionApp] = useState<Application | null>(null);
  const [deleteApp, setDeleteApp] = useState<Application | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const {
    data: applications,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ["applications"],
    queryFn: () => api.get<Application[]>("/api/v1/applications"),
  });

  return (
    <div className="space-y-6">
      {/* Header */}
      <Card className="app-surface border-border/60">
        <CardContent className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <div className="mb-2 inline-flex items-center rounded-full border border-primary/15 bg-primary/8 px-3 py-1 text-xs font-medium text-primary">
              Pipeline Tracking
            </div>
            <h1 className="text-2xl font-bold tracking-tight">Applications</h1>
            <p className="mt-1 text-muted-foreground">
            Track and manage your job applications.
            </p>
          </div>
          <Button className="shadow-sm" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            New Application
          </Button>
        </CardContent>
      </Card>

      <Separator className="opacity-60" />

      {/* Content */}
      {isLoading && <TableSkeleton />}

      {isError && (
        <div className="rounded-2xl border border-destructive/40 bg-destructive/5 p-4 text-sm text-destructive">
          Failed to load applications: {(error as Error).message}
        </div>
      )}

      {applications && applications.length === 0 && (
        <div className="empty-state-card border-dashed">
          <p className="text-lg font-medium">No applications yet</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Get started by creating your first job application.
          </p>
          <Button className="mt-4 shadow-sm" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            New Application
          </Button>
        </div>
      )}

      {applications && applications.length > 0 && (
        <div className="data-table-card">
          <div className="flex items-center justify-between border-b border-border/60 bg-background/45 px-4 py-3">
            <div>
              <p className="text-sm font-semibold">Applications List</p>
              <p className="text-xs text-muted-foreground">
                {applications.length} item{applications.length !== 1 ? "s" : ""}
              </p>
            </div>
          </div>
          <div className="data-table-wrap">
          <table className="data-table">
            <thead>
              <tr>
                <th className="!px-3">Company</th>
                <th className="!px-3">Position</th>
                <th className="!px-3">Status</th>
                <th className="!px-3">Applied</th>
                <th className="!px-3 text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {applications.map((app) => (
                <ApplicationRow
                  key={app.id}
                  application={app}
                  expanded={expandedId === app.id}
                  onToggleExpand={() =>
                    setExpandedId(expandedId === app.id ? null : app.id)
                  }
                  onEdit={() => setEditApp(app)}
                  onTransition={() => setTransitionApp(app)}
                  onDelete={() => setDeleteApp(app)}
                />
              ))}
            </tbody>
          </table>
          </div>
        </div>
      )}

      {/* Dialogs */}
      {createOpen && (
        <ApplicationFormDialog
          open={createOpen}
          onOpenChange={(open) => {
            setCreateOpen(open);
          }}
          mode="create"
        />
      )}

      {editApp && (
        <ApplicationFormDialog
          key={editApp.id}
          open={true}
          onOpenChange={(open) => {
            if (!open) setEditApp(null);
          }}
          initialData={editApp}
          mode="edit"
        />
      )}

      {transitionApp && (
        <TransitionDialog
          key={transitionApp.id}
          open={true}
          onOpenChange={(open) => {
            if (!open) setTransitionApp(null);
          }}
          application={transitionApp}
        />
      )}

      {deleteApp && (
        <DeleteDialog
          key={deleteApp.id}
          open={true}
          onOpenChange={(open) => {
            if (!open) setDeleteApp(null);
          }}
          application={deleteApp}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Table Row (extracted for readability)
// ---------------------------------------------------------------------------

function ApplicationRow({
  application,
  expanded,
  onToggleExpand,
  onEdit,
  onTransition,
  onDelete,
}: {
  application: Application;
  expanded: boolean;
  onToggleExpand: () => void;
  onEdit: () => void;
  onTransition: () => void;
  onDelete: () => void;
}) {
  return (
    <>
      <tr
        className="cursor-pointer"
        onClick={onToggleExpand}
      >
        <td className="!px-3 font-medium">
          <div className="flex items-center gap-1.5">
            {application.company_name}
            {application.job_url && (
              <a
                href={application.job_url}
                target="_blank"
                rel="noopener noreferrer"
                onClick={(e) => e.stopPropagation()}
                className="rounded-md p-0.5 text-muted-foreground hover:bg-background/80 hover:text-foreground"
              >
                <ExternalLink className="h-3.5 w-3.5" />
              </a>
            )}
          </div>
        </td>
        <td className="!px-3">{application.position_title}</td>
        <td className="!px-3">
          <StatusBadge status={application.status} />
        </td>
        <td className="!px-3 text-muted-foreground">
          {formatDate(application.applied_at ?? application.created_at)}
        </td>
        <td className="!px-3 text-right">
          <div
            className="flex items-center justify-end gap-1"
            onClick={(e) => e.stopPropagation()}
          >
            <Button
              variant="ghost"
              size="sm"
              className="rounded-lg hover:bg-background/80"
              onClick={onTransition}
              title="Transition status"
            >
              <ArrowRight className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="rounded-lg hover:bg-background/80"
              onClick={onEdit}
              title="Edit"
            >
              <Pencil className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="rounded-lg hover:bg-destructive/8 hover:text-destructive"
              onClick={onDelete}
              title="Delete"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        </td>
      </tr>
      {expanded && (
        <tr>
          <td colSpan={5} className="bg-background/30">
            <div className="grid grid-cols-1 gap-0 divide-y divide-border/60 md:grid-cols-2 md:divide-x md:divide-y-0">
              {/* Details */}
              <div className="p-4 space-y-2">
                <h4 className="text-sm font-semibold">Details</h4>
                {application.notes && (
                  <p className="text-sm text-muted-foreground whitespace-pre-wrap">
                    {application.notes}
                  </p>
                )}
                {!application.notes && (
                  <p className="text-sm text-muted-foreground italic">
                    No notes.
                  </p>
                )}
                <div className="space-y-0.5 pt-2 text-xs text-muted-foreground">
                  <p>Created: {formatDate(application.created_at)}</p>
                  <p>Updated: {formatDate(application.updated_at)}</p>
                </div>
              </div>
              {/* History */}
              <HistoryPanel applicationId={application.id} />
            </div>
          </td>
        </tr>
      )}
    </>
  );
}
