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
import type { InterviewPlan } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const STATUSES = [
  "pending",
  "scheduled",
  "preparing",
  "ready",
  "completed",
  "cancelled",
] as const;

type InterviewStatus = (typeof STATUSES)[number];

function statusVariant(
  status: string
): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "completed":
      return "default";
    case "ready":
      return "default";
    case "scheduled":
    case "preparing":
      return "secondary";
    case "cancelled":
      return "destructive";
    case "pending":
    default:
      return "outline";
  }
}

interface InterviewForm {
  company_name: string;
  position_title: string;
  interview_date: string;
  prep_materials: string;
}

const emptyForm: InterviewForm = {
  company_name: "",
  position_title: "",
  interview_date: "",
  prep_materials: "",
};

export default function Interviews() {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingInterview, setEditingInterview] =
    useState<InterviewPlan | null>(null);
  const [form, setForm] = useState<InterviewForm>(emptyForm);
  const [deleteTarget, setDeleteTarget] = useState<InterviewPlan | null>(null);
  const [statusTarget, setStatusTarget] = useState<InterviewPlan | null>(null);
  const [selectedStatus, setSelectedStatus] = useState<InterviewStatus>("pending");

  const { data: interviews, isLoading } = useQuery({
    queryKey: ["interviews"],
    queryFn: () => api.get<InterviewPlan[]>("/api/v1/interviews"),
  });

  const createMutation = useMutation({
    mutationFn: (data: InterviewForm) =>
      api.post<InterviewPlan>("/api/v1/interviews", {
        company_name: data.company_name,
        position_title: data.position_title,
        interview_date: data.interview_date || null,
        prep_materials: data.prep_materials || null,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["interviews"] });
      closeDialog();
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: string;
      data: Partial<InterviewForm>;
    }) =>
      api.put<InterviewPlan>(`/api/v1/interviews/${id}`, {
        company_name: data.company_name || undefined,
        position_title: data.position_title || undefined,
        interview_date: data.interview_date || null,
        prep_materials: data.prep_materials || null,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["interviews"] });
      closeDialog();
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.del(`/api/v1/interviews/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["interviews"] });
      setDeleteTarget(null);
    },
  });

  const statusMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: string }) =>
      api.post<InterviewPlan>(`/api/v1/interviews/${id}/status`, { status }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["interviews"] });
      setStatusTarget(null);
    },
  });

  const prepMutation = useMutation({
    mutationFn: (id: string) =>
      api.post<InterviewPlan>(`/api/v1/interviews/${id}/prep`, {}),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["interviews"] });
    },
  });

  function openCreate() {
    setEditingInterview(null);
    setForm(emptyForm);
    setDialogOpen(true);
  }

  function openEdit(interview: InterviewPlan) {
    setEditingInterview(interview);
    setForm({
      company_name: interview.company_name,
      position_title: interview.position_title,
      interview_date: interview.interview_date
        ? interview.interview_date.slice(0, 10)
        : "",
      prep_materials: interview.prep_materials ?? "",
    });
    setDialogOpen(true);
  }

  function openStatusChange(interview: InterviewPlan) {
    setStatusTarget(interview);
    setSelectedStatus(interview.status as InterviewStatus);
  }

  function closeDialog() {
    setDialogOpen(false);
    setEditingInterview(null);
    setForm(emptyForm);
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!form.company_name.trim() || !form.position_title.trim()) return;
    if (editingInterview) {
      updateMutation.mutate({ id: editingInterview.id, data: form });
    } else {
      createMutation.mutate(form);
    }
  }

  function handleStatusSubmit() {
    if (!statusTarget) return;
    statusMutation.mutate({ id: statusTarget.id, status: selectedStatus });
  }

  const isSaving = createMutation.isPending || updateMutation.isPending;

  function formatDate(dateStr: string | null) {
    if (!dateStr) return "-";
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
          <h1 className="text-2xl font-bold">Interviews</h1>
          <p className="text-muted-foreground mt-1">
            Prepare for and track your interviews.
          </p>
        </div>
        <Button onClick={openCreate}>New Interview</Button>
      </div>

      {isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={i} className="h-14 w-full" />
          ))}
        </div>
      ) : !interviews || interviews.length === 0 ? (
        <div className="rounded-lg border border-dashed p-8 text-center">
          <p className="text-muted-foreground">
            No interviews yet. Add your first interview to start tracking.
          </p>
        </div>
      ) : (
        <div className="rounded-lg border">
          <table className="w-full">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Company
                </th>
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Position
                </th>
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Status
                </th>
                <th className="px-4 py-3 text-left text-sm font-medium">
                  Interview Date
                </th>
                <th className="px-4 py-3 text-right text-sm font-medium">
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {interviews.map((interview) => (
                <tr key={interview.id} className="border-b last:border-b-0">
                  <td className="px-4 py-3 text-sm font-medium">
                    {interview.company_name}
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground">
                    {interview.position_title}
                  </td>
                  <td className="px-4 py-3 text-sm">
                    <Badge variant={statusVariant(interview.status)}>
                      {interview.status}
                    </Badge>
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground">
                    {formatDate(interview.interview_date)}
                  </td>
                  <td className="px-4 py-3 text-right">
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => openEdit(interview)}
                      >
                        Edit
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => openStatusChange(interview)}
                      >
                        Status
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={prepMutation.isPending}
                        onClick={() => prepMutation.mutate(interview.id)}
                      >
                        {prepMutation.isPending ? "Generating..." : "Prep"}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="text-destructive"
                        onClick={() => setDeleteTarget(interview)}
                      >
                        Delete
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Create / Edit Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {editingInterview ? "Edit Interview" : "New Interview"}
            </DialogTitle>
            <DialogDescription>
              {editingInterview
                ? "Update the interview details below."
                : "Fill in the details to track a new interview."}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="company_name">Company Name *</Label>
              <Input
                id="company_name"
                value={form.company_name}
                onChange={(e) =>
                  setForm((f) => ({ ...f, company_name: e.target.value }))
                }
                placeholder="e.g. Acme Corp"
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="position_title">Position Title *</Label>
              <Input
                id="position_title"
                value={form.position_title}
                onChange={(e) =>
                  setForm((f) => ({ ...f, position_title: e.target.value }))
                }
                placeholder="e.g. Senior Software Engineer"
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="interview_date">Interview Date</Label>
              <Input
                id="interview_date"
                type="date"
                value={form.interview_date}
                onChange={(e) =>
                  setForm((f) => ({ ...f, interview_date: e.target.value }))
                }
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="prep_materials">Prep Materials</Label>
              <Textarea
                id="prep_materials"
                value={form.prep_materials}
                onChange={(e) =>
                  setForm((f) => ({ ...f, prep_materials: e.target.value }))
                }
                placeholder="Notes, questions to prepare, topics to review..."
                rows={6}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={closeDialog}>
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={
                  isSaving ||
                  !form.company_name.trim() ||
                  !form.position_title.trim()
                }
              >
                {isSaving
                  ? "Saving..."
                  : editingInterview
                    ? "Update"
                    : "Create"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Status Change Dialog */}
      <Dialog
        open={statusTarget !== null}
        onOpenChange={(open) => {
          if (!open) setStatusTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Update Status</DialogTitle>
            <DialogDescription>
              Change the status for the interview at{" "}
              {statusTarget?.company_name} - {statusTarget?.position_title}.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label>Status</Label>
              <Select
                value={selectedStatus}
                onValueChange={(v) => setSelectedStatus(v as InterviewStatus)}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select status" />
                </SelectTrigger>
                <SelectContent>
                  {STATUSES.map((s) => (
                    <SelectItem key={s} value={s}>
                      {s}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <DialogFooter>
              <Button
                variant="outline"
                onClick={() => setStatusTarget(null)}
              >
                Cancel
              </Button>
              <Button
                disabled={statusMutation.isPending}
                onClick={handleStatusSubmit}
              >
                {statusMutation.isPending ? "Updating..." : "Update Status"}
              </Button>
            </DialogFooter>
          </div>
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
            <DialogTitle>Delete Interview</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete the interview at "
              {deleteTarget?.company_name}"? This action cannot be undone.
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
