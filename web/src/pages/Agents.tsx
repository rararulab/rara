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
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { fetchAgents, createAgent, deleteAgent } from "@/api/agents";
import { settingsApi } from "@/api/client";
import type { AgentResponse, CreateAgentRequest } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Bot,
  Lock,
  Plus,
  Settings2,
  Trash2,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Role badge variant helper
// ---------------------------------------------------------------------------

function roleBadgeVariant(
  role: string | null
): "default" | "secondary" | "outline" {
  switch (role) {
    case "Chat":
      return "default";
    case "Scout":
    case "Planner":
      return "secondary";
    case "Worker":
      return "outline";
    default:
      return "outline";
  }
}

// ---------------------------------------------------------------------------
// Create Agent Dialog
// ---------------------------------------------------------------------------

interface AgentFormData {
  name: string;
  description: string;
  model: string;
  system_prompt: string;
  soul_prompt: string;
  provider_hint: string;
  max_iterations: string;
  tools: string;
}

const EMPTY_FORM: AgentFormData = {
  name: "",
  description: "",
  model: "",
  system_prompt: "",
  soul_prompt: "",
  provider_hint: "",
  max_iterations: "",
  tools: "",
};

function CreateAgentDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<AgentFormData>({ ...EMPTY_FORM });

  const mutation = useMutation({
    mutationFn: (data: AgentFormData): Promise<AgentResponse> => {
      const req: CreateAgentRequest = {
        name: data.name.trim(),
        description: data.description.trim(),
        model: data.model.trim(),
        system_prompt: data.system_prompt.trim(),
      };
      if (data.soul_prompt.trim()) {
        req.soul_prompt = data.soul_prompt.trim();
      }
      if (data.provider_hint) {
        req.provider_hint = data.provider_hint;
      }
      const maxIter = parseInt(data.max_iterations, 10);
      if (!isNaN(maxIter) && maxIter > 0) {
        req.max_iterations = maxIter;
      }
      const tools = data.tools
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean);
      if (tools.length > 0) {
        req.tools = tools;
      }
      return createAgent(req);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      setForm({ ...EMPTY_FORM });
      onOpenChange(false);
    },
  });

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    mutation.mutate(form);
  }

  function updateField<K extends keyof AgentFormData>(
    key: K,
    value: AgentFormData[K]
  ) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px] max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Create Agent</DialogTitle>
          <DialogDescription>
            Define a new custom agent with its own model, prompts, and tools.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={handleSubmit}
          className="grid gap-4 py-4 overflow-y-auto"
        >
          <div className="grid gap-2">
            <Label htmlFor="agent-name">Name *</Label>
            <Input
              id="agent-name"
              value={form.name}
              onChange={(e) => updateField("name", e.target.value)}
              placeholder="e.g. code-reviewer"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-description">Description *</Label>
            <Input
              id="agent-description"
              value={form.description}
              onChange={(e) => updateField("description", e.target.value)}
              placeholder="e.g. Reviews code changes for quality"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-model">Model *</Label>
            <Input
              id="agent-model"
              value={form.model}
              onChange={(e) => updateField("model", e.target.value)}
              placeholder="e.g. openai/gpt-4o"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-system-prompt">System Prompt *</Label>
            <Textarea
              id="agent-system-prompt"
              value={form.system_prompt}
              onChange={(e) => updateField("system_prompt", e.target.value)}
              placeholder="You are a helpful assistant that..."
              rows={4}
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-soul-prompt">Soul Prompt</Label>
            <Textarea
              id="agent-soul-prompt"
              value={form.soul_prompt}
              onChange={(e) => updateField("soul_prompt", e.target.value)}
              placeholder="Optional personality / behavioral guidelines..."
              rows={3}
            />
            <p className="text-xs text-muted-foreground">
              Optional. Prepended before the system prompt.
            </p>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-provider">Provider Hint</Label>
            <Select
              value={form.provider_hint}
              onValueChange={(v) =>
                updateField("provider_hint", v === "__none__" ? "" : v)
              }
            >
              <SelectTrigger id="agent-provider">
                <SelectValue placeholder="Use default provider" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">Use default</SelectItem>
                <SelectItem value="openrouter">openrouter</SelectItem>
                <SelectItem value="ollama">ollama</SelectItem>
                <SelectItem value="codex">codex</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-max-iterations">Max Iterations</Label>
            <Input
              id="agent-max-iterations"
              type="number"
              min={1}
              value={form.max_iterations}
              onChange={(e) => updateField("max_iterations", e.target.value)}
              placeholder="e.g. 10"
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-tools">Tools</Label>
            <Input
              id="agent-tools"
              value={form.tools}
              onChange={(e) => updateField("tools", e.target.value)}
              placeholder="e.g. Read, Write, Bash (comma-separated)"
            />
            <p className="text-xs text-muted-foreground">
              Comma-separated list of tool names.
            </p>
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
              {mutation.isPending ? "Creating..." : "Create"}
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
// Configure Agent Dialog (per-agent provider/model override via settings)
// ---------------------------------------------------------------------------

function ConfigureAgentDialog({
  open,
  onOpenChange,
  agent,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  agent: AgentResponse;
}) {
  const queryClient = useQueryClient();
  const providerKey = `llm.agents.${agent.name}.provider`;
  const modelKey = `llm.agents.${agent.name}.model`;

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => settingsApi.list(),
  });

  const currentProvider = settingsQuery.data?.[providerKey] ?? "";
  const currentModel = settingsQuery.data?.[modelKey] ?? "";

  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  const [initialized, setInitialized] = useState(false);

  // Sync from loaded settings once
  if (settingsQuery.data && !initialized) {
    setProvider(currentProvider);
    setModel(currentModel);
    setInitialized(true);
  }

  // Reset when dialog opens for a different agent
  const handleOpenChange = (isOpen: boolean) => {
    if (!isOpen) {
      setInitialized(false);
    }
    onOpenChange(isOpen);
  };

  const saveMutation = useMutation({
    mutationFn: async () => {
      const patches: Record<string, string | null> = {};
      if (provider !== currentProvider) {
        patches[providerKey] = provider || null;
      }
      if (model !== currentModel) {
        patches[modelKey] = model || null;
      }
      if (Object.keys(patches).length > 0) {
        await settingsApi.batchUpdate(patches);
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      handleOpenChange(false);
    },
  });

  const hasChanges = provider !== currentProvider || model !== currentModel;

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Configure: {agent.name}</DialogTitle>
          <DialogDescription>
            Override the provider and model for this agent. Leave empty to use
            the global default.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 py-4">
          <div className="space-y-1.5">
            <Label htmlFor="cfg-provider">Provider Override</Label>
            <Select
              value={provider || "__none__"}
              onValueChange={(v) => setProvider(v === "__none__" ? "" : v)}
            >
              <SelectTrigger id="cfg-provider">
                <SelectValue placeholder="Use default provider" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">Use default</SelectItem>
                <SelectItem value="openrouter">openrouter</SelectItem>
                <SelectItem value="ollama">ollama</SelectItem>
                <SelectItem value="codex">codex</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground font-mono">
              {providerKey}
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="cfg-model">Model Override</Label>
            <Input
              id="cfg-model"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="e.g. openai/gpt-4o (leave empty for default)"
              className="font-mono text-sm"
            />
            <p className="text-xs text-muted-foreground font-mono">
              {modelKey}
            </p>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => handleOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={() => saveMutation.mutate()}
            disabled={!hasChanges || saveMutation.isPending}
          >
            {saveMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </DialogFooter>
        {saveMutation.isError && (
          <p className="text-sm text-destructive mt-2">
            Error: {(saveMutation.error as Error).message}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Delete Confirmation Dialog
// ---------------------------------------------------------------------------

function DeleteAgentDialog({
  open,
  onOpenChange,
  agent,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  agent: AgentResponse;
}) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => deleteAgent(agent.name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      onOpenChange(false);
    },
  });

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete Agent</AlertDialogTitle>
          <AlertDialogDescription>
            Are you sure you want to delete the agent{" "}
            <strong>{agent.name}</strong>? This action cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending}
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            {mutation.isPending ? "Deleting..." : "Delete"}
          </AlertDialogAction>
        </AlertDialogFooter>
        {mutation.isError && (
          <p className="text-sm text-destructive mt-2">
            Error: {(mutation.error as Error).message}
          </p>
        )}
      </AlertDialogContent>
    </AlertDialog>
  );
}

// ---------------------------------------------------------------------------
// Agent Card
// ---------------------------------------------------------------------------

function AgentCard({
  agent,
  onConfigure,
  onDelete,
}: {
  agent: AgentResponse;
  onConfigure: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="rounded-lg border bg-card p-4 space-y-3">
      {/* Header */}
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <Bot className="h-5 w-5 text-muted-foreground shrink-0" />
          <h3 className="font-semibold text-lg truncate">{agent.name}</h3>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          {agent.builtin && (
            <Badge variant="secondary" className="text-xs gap-1">
              <Lock className="h-3 w-3" />
              Built-in
            </Badge>
          )}
          {agent.role && (
            <Badge variant={roleBadgeVariant(agent.role)} className="text-xs">
              {agent.role}
            </Badge>
          )}
        </div>
      </div>

      {/* Description */}
      <p className="text-sm text-muted-foreground line-clamp-2">
        {agent.description || "No description"}
      </p>

      {/* Model & Provider */}
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span>
          Model:{" "}
          <span className="font-mono">
            {agent.model ?? "Default"}
          </span>
        </span>
        <span>
          Provider:{" "}
          <span className="font-mono">
            {agent.provider_hint ?? "Default"}
          </span>
        </span>
        {agent.max_iterations != null && (
          <span>Max Iterations: {agent.max_iterations}</span>
        )}
      </div>

      {/* Tools */}
      {agent.tools.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {agent.tools.slice(0, 8).map((tool) => (
            <Badge key={tool} variant="outline" className="text-xs">
              {tool}
            </Badge>
          ))}
          {agent.tools.length > 8 && (
            <Badge variant="outline" className="text-xs">
              +{agent.tools.length - 8} more
            </Badge>
          )}
        </div>
      )}

      <Separator />

      {/* Footer: actions */}
      <div className="flex items-center justify-end gap-1">
        <Button
          variant="ghost"
          size="sm"
          onClick={onConfigure}
          title="Configure provider/model override"
        >
          <Settings2 className="h-4 w-4 mr-1" />
          Configure
        </Button>
        {!agent.builtin && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            title="Delete agent"
            className="text-muted-foreground hover:text-destructive"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Loading Skeleton
// ---------------------------------------------------------------------------

function AgentsSkeleton() {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
      {Array.from({ length: 3 }).map((_, i) => (
        <Skeleton key={i} className="h-52 rounded-lg" />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export default function Agents() {
  const [createOpen, setCreateOpen] = useState(false);
  const [configureAgent, setConfigureAgent] = useState<AgentResponse | null>(
    null
  );
  const [deleteAgentItem, setDeleteAgentItem] =
    useState<AgentResponse | null>(null);

  const {
    data: agents,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ["agents"],
    queryFn: fetchAgents,
  });

  // Sort: builtin first, then by name
  const sortedAgents = agents
    ? [...agents].sort((a, b) => {
        if (a.builtin !== b.builtin) return a.builtin ? -1 : 1;
        return a.name.localeCompare(b.name);
      })
    : [];

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Agents</h1>
          <p className="text-muted-foreground mt-1">
            Manage agent definitions and per-agent provider/model overrides.
          </p>
        </div>
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="h-4 w-4" />
          New Agent
        </Button>
      </div>

      <Separator />

      {/* Content */}
      {isLoading && <AgentsSkeleton />}

      {isError && (
        <div className="rounded-lg border border-destructive/50 p-4 text-sm text-destructive">
          Failed to load agents: {(error as Error).message}
        </div>
      )}

      {agents && agents.length === 0 && (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
          <Bot className="h-10 w-10 text-muted-foreground mb-3" />
          <p className="text-lg font-medium">No agents defined</p>
          <p className="text-sm text-muted-foreground mt-1">
            Get started by creating your first custom agent.
          </p>
          <Button className="mt-4" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            New Agent
          </Button>
        </div>
      )}

      {sortedAgents.length > 0 && (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {sortedAgents.map((agent) => (
            <AgentCard
              key={agent.name}
              agent={agent}
              onConfigure={() => setConfigureAgent(agent)}
              onDelete={() => setDeleteAgentItem(agent)}
            />
          ))}
        </div>
      )}

      {/* Dialogs */}
      {createOpen && (
        <CreateAgentDialog
          open={createOpen}
          onOpenChange={(open) => setCreateOpen(open)}
        />
      )}

      {configureAgent && (
        <ConfigureAgentDialog
          key={configureAgent.name}
          open={true}
          onOpenChange={(open) => {
            if (!open) setConfigureAgent(null);
          }}
          agent={configureAgent}
        />
      )}

      {deleteAgentItem && (
        <DeleteAgentDialog
          key={deleteAgentItem.name}
          open={true}
          onOpenChange={(open) => {
            if (!open) setDeleteAgentItem(null);
          }}
          agent={deleteAgentItem}
        />
      )}
    </div>
  );
}
