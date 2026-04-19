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

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Bot, Lock, Plus, Settings2, Trash2, FileText } from 'lucide-react';
import { useState, useEffect, useMemo } from 'react';

import { fetchAgents, createAgent, deleteAgent } from '@/api/agents';
import { settingsApi } from '@/api/client';
import type { AgentResponse, CreateAgentRequest } from '@/api/types';
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
import { Skeleton } from '@/components/ui/skeleton';
import { Textarea } from '@/components/ui/textarea';

// ---------------------------------------------------------------------------
// Role badge variant helper
// ---------------------------------------------------------------------------

function roleBadgeVariant(role: string | null): 'default' | 'secondary' | 'outline' {
  switch (role) {
    case 'Chat':
      return 'default';
    case 'Scout':
    case 'Planner':
      return 'secondary';
    case 'Worker':
      return 'outline';
    default:
      return 'outline';
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
  name: '',
  description: '',
  model: '',
  system_prompt: '',
  soul_prompt: '',
  provider_hint: '',
  max_iterations: '',
  tools: '',
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
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean);
      if (tools.length > 0) {
        req.tools = tools;
      }
      return createAgent(req);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['agents'] });
      setForm({ ...EMPTY_FORM });
      onOpenChange(false);
    },
  });

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    mutation.mutate(form);
  }

  function updateField<K extends keyof AgentFormData>(key: K, value: AgentFormData[K]) {
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
        <form onSubmit={handleSubmit} className="grid gap-4 py-4 overflow-y-auto">
          <div className="grid gap-2">
            <Label htmlFor="agent-name">Name *</Label>
            <Input
              id="agent-name"
              value={form.name}
              onChange={(e) => updateField('name', e.target.value)}
              placeholder="e.g. code-reviewer"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-description">Description *</Label>
            <Input
              id="agent-description"
              value={form.description}
              onChange={(e) => updateField('description', e.target.value)}
              placeholder="e.g. Reviews code changes for quality"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-model">Model *</Label>
            <Input
              id="agent-model"
              value={form.model}
              onChange={(e) => updateField('model', e.target.value)}
              placeholder="e.g. openai/gpt-4o"
              required
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-system-prompt">System Prompt *</Label>
            <Textarea
              id="agent-system-prompt"
              value={form.system_prompt}
              onChange={(e) => updateField('system_prompt', e.target.value)}
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
              onChange={(e) => updateField('soul_prompt', e.target.value)}
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
              onValueChange={(v) => updateField('provider_hint', v === '__none__' ? '' : v)}
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
              onChange={(e) => updateField('max_iterations', e.target.value)}
              placeholder="e.g. 10"
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="agent-tools">Tools</Label>
            <Input
              id="agent-tools"
              value={form.tools}
              onChange={(e) => updateField('tools', e.target.value)}
              placeholder="e.g. Read, Write, Bash (comma-separated)"
            />
            <p className="text-xs text-muted-foreground">Comma-separated list of tool names.</p>
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
            <p className="text-sm text-destructive">Error: {mutation.error.message}</p>
          )}
        </form>
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
      void queryClient.invalidateQueries({ queryKey: ['agents'] });
      onOpenChange(false);
    },
  });

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete Agent</AlertDialogTitle>
          <AlertDialogDescription>
            Are you sure you want to delete the agent <strong>{agent.name}</strong>? This action
            cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending}
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            {mutation.isPending ? 'Deleting...' : 'Delete'}
          </AlertDialogAction>
        </AlertDialogFooter>
        {mutation.isError && (
          <p className="text-sm text-destructive mt-2">Error: {mutation.error.message}</p>
        )}
      </AlertDialogContent>
    </AlertDialog>
  );
}

// ---------------------------------------------------------------------------
// Agent List Item (left panel)
// ---------------------------------------------------------------------------

function AgentListItem({
  agent,
  isSelected,
  onClick,
}: {
  agent: AgentResponse;
  isSelected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex w-full items-center gap-3 px-4 py-3 text-left transition-colors ${
        isSelected ? 'bg-accent' : 'hover:bg-accent/50'
      }`}
    >
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-muted">
        <Bot className="h-4 w-4 text-muted-foreground" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{agent.name}</span>
          {agent.builtin && <Lock className="h-3 w-3 text-muted-foreground shrink-0" />}
        </div>
        <div className="flex items-center gap-1.5 mt-0.5">
          <span className="text-xs text-muted-foreground">{agent.role ?? 'Agent'}</span>
        </div>
      </div>
    </button>
  );
}

// ---------------------------------------------------------------------------
// Overview Tab (right panel)
// ---------------------------------------------------------------------------

function OverviewTab({ agent }: { agent: AgentResponse }) {
  return (
    <div className="space-y-6">
      <div className="space-y-1.5">
        <h3 className="text-sm font-medium">Description</h3>
        <p className="text-sm text-muted-foreground">{agent.description || 'No description'}</p>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <h3 className="text-sm font-medium">Model</h3>
          <p className="text-sm text-muted-foreground font-mono">{agent.model ?? 'Default'}</p>
        </div>
        <div className="space-y-1.5">
          <h3 className="text-sm font-medium">Provider</h3>
          <p className="text-sm text-muted-foreground font-mono">
            {agent.provider_hint ?? 'Default'}
          </p>
        </div>
      </div>

      {agent.max_iterations != null && (
        <div className="space-y-1.5">
          <h3 className="text-sm font-medium">Max Iterations</h3>
          <p className="text-sm text-muted-foreground">{agent.max_iterations}</p>
        </div>
      )}

      <div className="space-y-2">
        <h3 className="text-sm font-medium">Tools</h3>
        {agent.tools.length > 0 ? (
          <div className="flex flex-wrap gap-1.5">
            {agent.tools.map((tool) => (
              <Badge key={tool} variant="outline" className="text-xs">
                {tool}
              </Badge>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">No tools assigned</p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Configure Tab (right panel)
// ---------------------------------------------------------------------------

function ConfigureTab({ agent }: { agent: AgentResponse }) {
  const queryClient = useQueryClient();
  const providerKey = `llm.agents.${agent.name}.provider`;
  const modelKey = `llm.agents.${agent.name}.model`;

  const settingsQuery = useQuery({
    queryKey: ['settings'],
    queryFn: () => settingsApi.list(),
  });

  const currentProvider = settingsQuery.data?.[providerKey] ?? '';
  const currentModel = settingsQuery.data?.[modelKey] ?? '';

  const [provider, setProvider] = useState('');
  const [model, setModel] = useState('');
  const [initialized, setInitialized] = useState(false);

  if (settingsQuery.data && !initialized) {
    setProvider(currentProvider);
    setModel(currentModel);
    setInitialized(true);
  }

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
      void queryClient.invalidateQueries({ queryKey: ['settings'] });
    },
  });

  const hasChanges = provider !== currentProvider || model !== currentModel;

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-sm font-medium">Provider / Model Override</h3>
        <p className="text-sm text-muted-foreground mt-1">
          Override the provider and model for this agent. Leave empty to use the global default.
        </p>
      </div>
      <div className="space-y-4 max-w-md">
        <div className="space-y-1.5">
          <Label htmlFor="cfg-provider">Provider Override</Label>
          <Select
            value={provider || '__none__'}
            onValueChange={(v) => setProvider(v === '__none__' ? '' : v)}
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
          <p className="text-xs text-muted-foreground font-mono">{providerKey}</p>
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
          <p className="text-xs text-muted-foreground font-mono">{modelKey}</p>
        </div>
        <Button
          onClick={() => saveMutation.mutate()}
          disabled={!hasChanges || saveMutation.isPending}
        >
          {saveMutation.isPending ? 'Saving...' : 'Save Changes'}
        </Button>
        {saveMutation.isError && (
          <p className="text-sm text-destructive">Error: {saveMutation.error.message}</p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Agent Detail (right panel)
// ---------------------------------------------------------------------------

type DetailTab = 'overview' | 'configure';

const detailTabs: { id: DetailTab; label: string; icon: typeof FileText }[] = [
  { id: 'overview', label: 'Overview', icon: FileText },
  { id: 'configure', label: 'Configure', icon: Settings2 },
];

function AgentDetail({ agent, onDelete }: { agent: AgentResponse; onDelete: () => void }) {
  const [activeTab, setActiveTab] = useState<DetailTab>('overview');

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="flex h-12 shrink-0 items-center gap-3 border-b px-4">
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted">
          <Bot className="h-4 w-4 text-muted-foreground" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold truncate">{agent.name}</h2>
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

      {/* Tabs */}
      <div className="flex border-b px-6">
        {detailTabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`flex items-center gap-1.5 border-b-2 px-3 py-2.5 text-xs font-medium transition-colors ${
              activeTab === tab.id
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            <tab.icon className="h-3.5 w-3.5" />
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab Content */}
      <div className="flex-1 overflow-y-auto p-6">
        {activeTab === 'overview' && <OverviewTab agent={agent} />}
        {activeTab === 'configure' && <ConfigureTab key={agent.name} agent={agent} />}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Loading Skeleton (two-panel)
// ---------------------------------------------------------------------------

function AgentsSkeleton() {
  return (
    <div className="flex flex-1 min-h-0">
      <div className="w-72 border-r">
        <div className="flex h-12 items-center justify-between border-b px-4">
          <Skeleton className="h-4 w-16" />
          <Skeleton className="h-6 w-6 rounded" />
        </div>
        <div className="divide-y">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="flex items-center gap-3 px-4 py-3">
              <Skeleton className="h-8 w-8 rounded-lg" />
              <div className="flex-1 space-y-1.5">
                <Skeleton className="h-4 w-24" />
                <Skeleton className="h-3 w-16" />
              </div>
            </div>
          ))}
        </div>
      </div>
      <div className="flex-1 p-6 space-y-6">
        <div className="flex items-center gap-3">
          <Skeleton className="h-7 w-7 rounded-md" />
          <Skeleton className="h-5 w-32" />
        </div>
        <Skeleton className="h-8 w-full rounded-lg" />
        <Skeleton className="h-8 w-full rounded-lg" />
        <Skeleton className="h-8 w-3/4 rounded-lg" />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export default function Agents() {
  const [createOpen, setCreateOpen] = useState(false);
  const [selectedName, setSelectedName] = useState<string>('');
  const [deleteAgentItem, setDeleteAgentItem] = useState<AgentResponse | null>(null);

  const {
    data: agents,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ['agents'],
    queryFn: fetchAgents,
  });

  const sortedAgents = useMemo(
    () =>
      agents
        ? [...agents].sort((a, b) => {
            if (a.builtin !== b.builtin) return a.builtin ? -1 : 1;
            return a.name.localeCompare(b.name);
          })
        : [],
    [agents],
  );

  // Auto-select first agent when list loads or selection becomes invalid
  useEffect(() => {
    const first = sortedAgents[0];
    if (first && !sortedAgents.some((a) => a.name === selectedName)) {
      setSelectedName(first.name);
    }
  }, [sortedAgents, selectedName]);

  const selected = sortedAgents.find((a) => a.name === selectedName) ?? null;

  if (isLoading) {
    return <AgentsSkeleton />;
  }

  if (isError) {
    return (
      <div className="p-6">
        <div className="rounded-lg border border-destructive/50 p-4 text-sm text-destructive">
          Failed to load agents: {error.message}
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-1 min-h-0">
      {/* Left panel — agent list */}
      <div className="w-72 shrink-0 overflow-y-auto border-r">
        <div className="flex h-12 items-center justify-between border-b px-4">
          <h1 className="text-sm font-semibold">Agents</h1>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0"
            onClick={() => setCreateOpen(true)}
          >
            <Plus className="h-4 w-4 text-muted-foreground" />
          </Button>
        </div>

        {sortedAgents.length === 0 ? (
          <div className="flex flex-col items-center justify-center px-4 py-12">
            <Bot className="h-8 w-8 text-muted-foreground/40" />
            <p className="mt-3 text-sm text-muted-foreground">No agents yet</p>
            <Button onClick={() => setCreateOpen(true)} size="sm" className="mt-3">
              <Plus className="h-3 w-3" />
              Create Agent
            </Button>
          </div>
        ) : (
          <div className="divide-y">
            {sortedAgents.map((agent) => (
              <AgentListItem
                key={agent.name}
                agent={agent}
                isSelected={agent.name === selectedName}
                onClick={() => setSelectedName(agent.name)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Right panel — agent detail */}
      <div className="flex-1 min-w-0">
        {selected ? (
          <AgentDetail
            key={selected.name}
            agent={selected}
            onDelete={() => setDeleteAgentItem(selected)}
          />
        ) : (
          <div className="flex h-full flex-col items-center justify-center text-muted-foreground">
            <Bot className="h-10 w-10 text-muted-foreground/30" />
            <p className="mt-3 text-sm">Select an agent to view details</p>
            <Button onClick={() => setCreateOpen(true)} size="sm" className="mt-3">
              <Plus className="h-3 w-3" />
              Create Agent
            </Button>
          </div>
        )}
      </div>

      {/* Dialogs */}
      {createOpen && (
        <CreateAgentDialog open={createOpen} onOpenChange={(open) => setCreateOpen(open)} />
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
