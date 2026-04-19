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

import { useCallback, useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/api/client';
import type {
  McpServerInfo,
  McpToolView,
  McpResourceView,
  McpLogEntry,
  CreateMcpServerRequest,
} from '@/api/types';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Skeleton } from '@/components/ui/skeleton';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FileText,
  Pencil,
  Play,
  Plus,
  RotateCcw,
  ScrollText,
  Server,
  Square,
  Trash2,
  Wrench,
} from 'lucide-react';

type ToastState = { kind: 'success' | 'error'; message: string } | null;

interface EnvEntry {
  key: string;
  value: string;
}

const EMPTY_FORM: FormState = {
  name: '',
  transport: 'stdio',
  command: '',
  args: '',
  url: '',
  env: [],
  enabled: true,
  startupTimeout: '',
  toolTimeout: '',
};

interface FormState {
  name: string;
  transport: string;
  command: string;
  args: string;
  url: string;
  env: EnvEntry[];
  enabled: boolean;
  startupTimeout: string;
  toolTimeout: string;
}

function statusBadge(status: McpServerInfo['status']) {
  switch (status.type) {
    case 'connected':
      return (
        <Badge className="border-transparent bg-green-100 text-green-800 hover:bg-green-100">
          Connected
        </Badge>
      );
    case 'connecting':
      return (
        <Badge variant="outline" className="bg-blue-50 text-blue-700 border-blue-200">
          Connecting...
        </Badge>
      );
    case 'disconnected':
      return <Badge variant="secondary">Disconnected</Badge>;
    case 'error':
      return <Badge variant="destructive">Error</Badge>;
  }
}

function formToRequest(form: FormState): CreateMcpServerRequest {
  const env: Record<string, string> = {};
  for (const entry of form.env) {
    const k = entry.key.trim();
    if (k) env[k] = entry.value;
  }
  const req: CreateMcpServerRequest = {
    name: form.name.trim(),
    command: form.command.trim(),
    args: form.args
      .split('\n')
      .map((s) => s.trim())
      .filter(Boolean),
    env,
    enabled: form.enabled,
    transport: form.transport,
  };
  if (form.transport === 'sse' && form.url.trim()) {
    req.url = form.url.trim();
  }
  const startupSecs = Number.parseInt(form.startupTimeout, 10);
  if (Number.isFinite(startupSecs) && startupSecs > 0) {
    req.startup_timeout_secs = startupSecs;
  }
  const toolSecs = Number.parseInt(form.toolTimeout, 10);
  if (Number.isFinite(toolSecs) && toolSecs > 0) {
    req.tool_timeout_secs = toolSecs;
  }
  return req;
}

function serverToForm(server: McpServerInfo): FormState {
  return {
    name: server.name,
    transport: server.config.transport,
    command: server.config.command,
    args: server.config.args.join('\n'),
    url: server.config.url ?? '',
    env: Object.entries(server.config.env).map(([key, value]) => ({
      key,
      value,
    })),
    enabled: server.config.enabled,
    startupTimeout: server.config.startup_timeout_secs?.toString() ?? '',
    toolTimeout: server.config.tool_timeout_secs?.toString() ?? '',
  };
}

export default function McpServers() {
  const queryClient = useQueryClient();
  const [toast, setToast] = useState<ToastState>(null);
  const [expandedServer, setExpandedServer] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingServer, setEditingServer] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>({ ...EMPTY_FORM });

  // ── Queries ──────────────────────────────────────────────

  const serversQuery = useQuery({
    queryKey: ['mcp-servers'],
    queryFn: () => api.get<McpServerInfo[]>('/api/v1/mcp/servers'),
    refetchInterval: 5000, // Poll every 5s for status changes
  });

  // ── Mutations ────────────────────────────────────────────

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['mcp-servers'] });

  const addMutation = useMutation({
    mutationFn: (req: CreateMcpServerRequest) =>
      api.post<McpServerInfo>('/api/v1/mcp/servers', req),
    onSuccess: () => {
      invalidate();
      setDialogOpen(false);
      setForm({ ...EMPTY_FORM });
      showToast('success', 'Server added successfully.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to add server'),
  });

  const updateMutation = useMutation({
    mutationFn: ({ name, req }: { name: string; req: CreateMcpServerRequest }) =>
      api.put<McpServerInfo>(`/api/v1/mcp/servers/${name}`, req),
    onSuccess: () => {
      invalidate();
      setDialogOpen(false);
      setEditingServer(null);
      setForm({ ...EMPTY_FORM });
      showToast('success', 'Server updated successfully.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to update server'),
  });

  const deleteMutation = useMutation({
    mutationFn: (name: string) => api.del<void>(`/api/v1/mcp/servers/${name}`),
    onSuccess: () => {
      invalidate();
      showToast('success', 'Server deleted.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to delete server'),
  });

  const startMutation = useMutation({
    mutationFn: (name: string) => api.post<void>(`/api/v1/mcp/servers/${name}/start`),
    onSuccess: () => {
      invalidate();
      showToast('success', 'Server started.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to start server'),
  });

  const stopMutation = useMutation({
    mutationFn: (name: string) => api.post<void>(`/api/v1/mcp/servers/${name}/stop`),
    onSuccess: () => {
      invalidate();
      showToast('success', 'Server stopped.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to stop server'),
  });

  const restartMutation = useMutation({
    mutationFn: (name: string) => api.post<void>(`/api/v1/mcp/servers/${name}/restart`),
    onSuccess: () => {
      invalidate();
      showToast('success', 'Server restarted.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to restart server'),
  });

  const enableMutation = useMutation({
    mutationFn: (name: string) => api.post<void>(`/api/v1/mcp/servers/${name}/enable`),
    onSuccess: () => {
      invalidate();
      showToast('success', 'Server enabled.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to enable server'),
  });

  const disableMutation = useMutation({
    mutationFn: (name: string) => api.post<void>(`/api/v1/mcp/servers/${name}/disable`),
    onSuccess: () => {
      invalidate();
      showToast('success', 'Server disabled.');
    },
    onError: (e: unknown) =>
      showToast('error', e instanceof Error ? e.message : 'Failed to disable server'),
  });

  const anyMutating =
    addMutation.isPending ||
    updateMutation.isPending ||
    deleteMutation.isPending ||
    startMutation.isPending ||
    stopMutation.isPending ||
    restartMutation.isPending ||
    enableMutation.isPending ||
    disableMutation.isPending;

  // ── Helpers ──────────────────────────────────────────────

  const showToast = useCallback((kind: 'success' | 'error', message: string) => {
    setToast({ kind, message });
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 3000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const openAdd = () => {
    setEditingServer(null);
    setForm({ ...EMPTY_FORM });
    setDialogOpen(true);
  };

  const openEdit = (server: McpServerInfo) => {
    setEditingServer(server.name);
    setForm(serverToForm(server));
    setDialogOpen(true);
  };

  const handleSubmit = () => {
    const req = formToRequest(form);
    if (!req.name) {
      showToast('error', 'Server name is required.');
      return;
    }
    if (req.transport === 'stdio' && !req.command) {
      showToast('error', 'Command is required for stdio transport.');
      return;
    }
    if (req.transport === 'sse' && !req.url) {
      showToast('error', 'URL is required for SSE transport.');
      return;
    }
    if (editingServer) {
      updateMutation.mutate({ name: editingServer, req });
    } else {
      addMutation.mutate(req);
    }
  };

  const handleDelete = (name: string) => {
    if (!window.confirm(`Delete MCP server "${name}"?`)) return;
    deleteMutation.mutate(name);
  };

  const handleToggleEnabled = (server: McpServerInfo) => {
    if (server.config.enabled) {
      disableMutation.mutate(server.name);
    } else {
      enableMutation.mutate(server.name);
    }
  };

  const addEnvEntry = () => {
    setForm((prev) => ({
      ...prev,
      env: [...prev.env, { key: '', value: '' }],
    }));
  };

  const updateEnvEntry = (index: number, field: 'key' | 'value', value: string) => {
    setForm((prev) => ({
      ...prev,
      env: prev.env.map((entry, i) => (i === index ? { ...entry, [field]: value } : entry)),
    }));
  };

  const removeEnvEntry = (index: number) => {
    setForm((prev) => ({
      ...prev,
      env: prev.env.filter((_, i) => i !== index),
    }));
  };

  // ── Render ───────────────────────────────────────────────

  const servers = serversQuery.data ?? [];
  const connectedCount = servers.filter((s) => s.status.type === 'connected').length;
  const connectingCount = servers.filter((s) => s.status.type === 'connecting').length;

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold">MCP Servers</h2>
          <p className="text-sm text-muted-foreground">
            {connectedCount} connected
            {connectingCount > 0 ? ` / ${connectingCount} connecting` : ''} / {servers.length}{' '}
            configured
          </p>
        </div>
        <Button onClick={openAdd}>
          <Plus className="mr-2 h-4 w-4" />
          Add Server
        </Button>
      </div>

      {/* Loading */}
      {serversQuery.isLoading && (
        <div className="space-y-3">
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-24 w-full" />
        </div>
      )}

      {/* Error */}
      {serversQuery.isError && (
        <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-800">
          Failed to load MCP servers.{' '}
          {serversQuery.error instanceof Error ? serversQuery.error.message : 'Unknown error'}
        </div>
      )}

      {/* Empty state */}
      {!serversQuery.isLoading && !serversQuery.isError && servers.length === 0 && (
        <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed p-12 text-center">
          <Server className="h-10 w-10 text-muted-foreground" />
          <p className="font-medium">No MCP servers configured</p>
          <p className="text-sm text-muted-foreground">
            Add an MCP server to extend the agent with external tools and resources.
          </p>
          <Button onClick={openAdd} variant="outline">
            <Plus className="mr-2 h-4 w-4" />
            Add Server
          </Button>
        </div>
      )}

      {/* Server cards */}
      {servers.map((server) => (
        <ServerCard
          key={server.name}
          server={server}
          expanded={expandedServer === server.name}
          onToggleExpand={() =>
            setExpandedServer((prev) => (prev === server.name ? null : server.name))
          }
          onEdit={() => openEdit(server)}
          onDelete={() => handleDelete(server.name)}
          onStart={() => startMutation.mutate(server.name)}
          onStop={() => stopMutation.mutate(server.name)}
          onRestart={() => restartMutation.mutate(server.name)}
          onToggleEnabled={() => handleToggleEnabled(server)}
          disabled={anyMutating}
        />
      ))}

      {/* Add/Edit Dialog */}
      <Dialog
        open={dialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            setDialogOpen(false);
            setEditingServer(null);
          }
        }}
      >
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>{editingServer ? `Edit: ${editingServer}` : 'Add MCP Server'}</DialogTitle>
            <DialogDescription>
              {editingServer
                ? 'Update the server configuration.'
                : 'Configure a new MCP server connection.'}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4 py-4">
            {/* Name */}
            <div className="space-y-2">
              <Label htmlFor="mcp-name">Name</Label>
              <Input
                id="mcp-name"
                value={form.name}
                onChange={(e) => setForm((prev) => ({ ...prev, name: e.target.value }))}
                placeholder="my-mcp-server"
                disabled={!!editingServer}
              />
            </div>

            {/* Transport */}
            <div className="space-y-2">
              <Label>Transport</Label>
              <Select
                value={form.transport}
                onValueChange={(value) => setForm((prev) => ({ ...prev, transport: value }))}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="stdio">stdio</SelectItem>
                  <SelectItem value="sse">SSE (HTTP)</SelectItem>
                </SelectContent>
              </Select>
            </div>

            {/* Command (stdio) */}
            {form.transport === 'stdio' && (
              <>
                <div className="space-y-2">
                  <Label htmlFor="mcp-command">Command</Label>
                  <Input
                    id="mcp-command"
                    value={form.command}
                    onChange={(e) =>
                      setForm((prev) => ({
                        ...prev,
                        command: e.target.value,
                      }))
                    }
                    placeholder="npx"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="mcp-args">
                    Arguments <span className="text-xs text-muted-foreground">(one per line)</span>
                  </Label>
                  <textarea
                    id="mcp-args"
                    value={form.args}
                    onChange={(e) => setForm((prev) => ({ ...prev, args: e.target.value }))}
                    placeholder={'-y\n@modelcontextprotocol/server-filesystem\n/path/to/dir'}
                    className="flex min-h-[80px] w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    rows={3}
                  />
                </div>
              </>
            )}

            {/* URL (sse) */}
            {form.transport === 'sse' && (
              <div className="space-y-2">
                <Label htmlFor="mcp-url">URL</Label>
                <Input
                  id="mcp-url"
                  value={form.url}
                  onChange={(e) => setForm((prev) => ({ ...prev, url: e.target.value }))}
                  placeholder="http://localhost:8080/sse"
                />
              </div>
            )}

            {/* Environment Variables */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>Environment Variables</Label>
                <Button type="button" variant="outline" size="sm" onClick={addEnvEntry}>
                  <Plus className="mr-1 h-3 w-3" />
                  Add
                </Button>
              </div>
              {form.env.length === 0 && (
                <p className="text-xs text-muted-foreground">
                  No environment variables configured.
                </p>
              )}
              {form.env.map((entry, index) => (
                <div key={index} className="flex items-center gap-2">
                  <Input
                    value={entry.key}
                    onChange={(e) => updateEnvEntry(index, 'key', e.target.value)}
                    placeholder="KEY"
                    className="flex-1"
                  />
                  <Input
                    value={entry.value}
                    onChange={(e) => updateEnvEntry(index, 'value', e.target.value)}
                    placeholder="value"
                    className="flex-1"
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-9 w-9 shrink-0 text-muted-foreground hover:text-destructive"
                    onClick={() => removeEnvEntry(index)}
                  >
                    <Trash2 className="h-3 w-3" />
                  </Button>
                </div>
              ))}
            </div>

            {/* Timeouts */}
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="mcp-startup-timeout">
                  Startup Timeout <span className="text-xs text-muted-foreground">(secs)</span>
                </Label>
                <Input
                  id="mcp-startup-timeout"
                  type="number"
                  value={form.startupTimeout}
                  onChange={(e) =>
                    setForm((prev) => ({
                      ...prev,
                      startupTimeout: e.target.value,
                    }))
                  }
                  placeholder="30"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="mcp-tool-timeout">
                  Tool Timeout <span className="text-xs text-muted-foreground">(secs)</span>
                </Label>
                <Input
                  id="mcp-tool-timeout"
                  type="number"
                  value={form.toolTimeout}
                  onChange={(e) =>
                    setForm((prev) => ({
                      ...prev,
                      toolTimeout: e.target.value,
                    }))
                  }
                  placeholder="60"
                />
              </div>
            </div>

            {/* Enabled */}
            <div className="flex items-center justify-between rounded-lg border px-4 py-3">
              <div>
                <p className="text-sm font-medium">Enabled</p>
                <p className="text-xs text-muted-foreground">
                  Server will auto-connect on startup when enabled.
                </p>
              </div>
              <Switch
                checked={form.enabled}
                onCheckedChange={(checked) => setForm((prev) => ({ ...prev, enabled: checked }))}
              />
            </div>
          </div>

          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setDialogOpen(false);
                setEditingServer(null);
              }}
              disabled={addMutation.isPending || updateMutation.isPending}
            >
              Cancel
            </Button>
            <Button
              onClick={handleSubmit}
              disabled={addMutation.isPending || updateMutation.isPending}
            >
              {addMutation.isPending || updateMutation.isPending
                ? 'Saving...'
                : editingServer
                  ? 'Update'
                  : 'Add Server'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Toast */}
      {toast && (
        <div className="fixed right-6 top-6 z-50">
          <div
            className={`rounded-md border px-4 py-3 text-sm shadow-lg ${
              toast.kind === 'success'
                ? 'border-green-200 bg-green-50 text-green-800'
                : 'border-red-200 bg-red-50 text-red-800'
            }`}
          >
            {toast.message}
          </div>
        </div>
      )}
    </div>
  );
}

// ── ServerCard component ────────────────────────────────────

function ServerCard({
  server,
  expanded,
  onToggleExpand,
  onEdit,
  onDelete,
  onStart,
  onStop,
  onRestart,
  onToggleEnabled,
  disabled,
}: {
  server: McpServerInfo;
  expanded: boolean;
  onToggleExpand: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onToggleEnabled: () => void;
  disabled: boolean;
}) {
  const isConnected = server.status.type === 'connected';
  const isConnecting = server.status.type === 'connecting';
  const isActive = isConnected || isConnecting;
  const summary =
    server.config.transport === 'sse'
      ? (server.config.url ?? 'SSE')
      : `${server.config.command} ${server.config.args.join(' ')}`;

  return (
    <div className="rounded-xl border bg-card shadow-sm">
      {/* Header row */}
      <button
        type="button"
        className="flex w-full items-center justify-between p-4 text-left transition-colors hover:bg-accent/50"
        onClick={onToggleExpand}
      >
        <div className="flex items-center gap-3 min-w-0">
          <Server className="h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <p className="font-medium">{server.name}</p>
              {statusBadge(server.status)}
              {!server.config.enabled && (
                <Badge variant="outline" className="text-xs">
                  Disabled
                </Badge>
              )}
            </div>
            <p className="truncate text-xs text-muted-foreground mt-0.5">
              {server.config.transport.toUpperCase()} -- {summary}
            </p>
          </div>
        </div>
        {expanded ? (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
      </button>

      {/* Expanded content */}
      {expanded && (
        <div className="border-t px-4 pb-4 pt-3 space-y-4">
          {/* Error message */}
          {server.status.type === 'error' && (
            <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800">
              {server.status.message}
            </div>
          )}

          {/* Action buttons */}
          <div className="flex flex-wrap items-center gap-2">
            {isActive ? (
              <Button variant="outline" size="sm" onClick={onStop} disabled={disabled}>
                <Square className="mr-1 h-3 w-3" />
                Stop
              </Button>
            ) : (
              <Button
                variant="outline"
                size="sm"
                onClick={onStart}
                disabled={disabled || !server.config.enabled}
              >
                <Play className="mr-1 h-3 w-3" />
                Start
              </Button>
            )}
            <Button
              variant="outline"
              size="sm"
              onClick={onRestart}
              disabled={disabled || !server.config.enabled}
            >
              <RotateCcw className="mr-1 h-3 w-3" />
              Restart
            </Button>
            <Button variant="outline" size="sm" onClick={onToggleEnabled} disabled={disabled}>
              {server.config.enabled ? 'Disable' : 'Enable'}
            </Button>
            <div className="flex-1" />
            <Button variant="outline" size="sm" onClick={onEdit} disabled={disabled}>
              <Pencil className="mr-1 h-3 w-3" />
              Edit
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="text-destructive hover:bg-destructive hover:text-destructive-foreground"
              onClick={onDelete}
              disabled={disabled}
            >
              <Trash2 className="mr-1 h-3 w-3" />
              Delete
            </Button>
          </div>

          {/* Config details */}
          <ConfigDetails server={server} />

          {/* Tools (only fetch when connected) */}
          {isConnected && <ToolsList serverName={server.name} />}

          {/* Resources (only fetch when connected) */}
          {isConnected && <ResourcesList serverName={server.name} />}

          {/* Logs (always available) */}
          <LogsList serverName={server.name} />
        </div>
      )}
    </div>
  );
}

// ── Config details ──────────────────────────────────────────

function ConfigDetails({ server }: { server: McpServerInfo }) {
  const config = server.config;
  return (
    <div className="space-y-2 rounded-lg border bg-muted/30 p-3">
      <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
        Configuration
      </p>
      <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
        <span className="text-muted-foreground">Transport</span>
        <span>{config.transport}</span>
        {config.command && (
          <>
            <span className="text-muted-foreground">Command</span>
            <span className="font-mono text-xs break-all">{config.command}</span>
          </>
        )}
        {config.args.length > 0 && (
          <>
            <span className="text-muted-foreground">Args</span>
            <span className="font-mono text-xs break-all">{config.args.join(' ')}</span>
          </>
        )}
        {config.url && (
          <>
            <span className="text-muted-foreground">URL</span>
            <span className="font-mono text-xs break-all">{config.url}</span>
          </>
        )}
        {config.startup_timeout_secs != null && (
          <>
            <span className="text-muted-foreground">Startup Timeout</span>
            <span>{config.startup_timeout_secs}s</span>
          </>
        )}
        {config.tool_timeout_secs != null && (
          <>
            <span className="text-muted-foreground">Tool Timeout</span>
            <span>{config.tool_timeout_secs}s</span>
          </>
        )}
        {Object.keys(config.env).length > 0 && (
          <>
            <span className="text-muted-foreground">Env Vars</span>
            <span>{Object.keys(config.env).length} configured</span>
          </>
        )}
        {config.tools_disabled.length > 0 && (
          <>
            <span className="text-muted-foreground">Disabled Tools</span>
            <span>{config.tools_disabled.join(', ')}</span>
          </>
        )}
      </div>
    </div>
  );
}

// ── Tools list ──────────────────────────────────────────────

function ToolsList({ serverName }: { serverName: string }) {
  const [expanded, setExpanded] = useState(false);
  const [expandedTool, setExpandedTool] = useState<string | null>(null);

  const toolsQuery = useQuery({
    queryKey: ['mcp-server-tools', serverName],
    queryFn: () => api.get<McpToolView[]>(`/api/v1/mcp/servers/${serverName}/tools`),
    enabled: expanded,
  });

  const tools = toolsQuery.data ?? [];

  return (
    <div className="rounded-lg border bg-muted/30">
      <button
        type="button"
        className="flex w-full items-center justify-between p-3 text-left transition-colors hover:bg-accent/50"
        onClick={() => setExpanded((v) => !v)}
      >
        <div className="flex items-center gap-2">
          <Wrench className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
            Tools
          </span>
          {expanded && !toolsQuery.isLoading && (
            <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
              {tools.length}
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-1">
          {expanded && (
            <a
              href={`/grafana/explore?orgId=1&left=${encodeURIComponent(
                JSON.stringify({
                  datasource: 'Quickwit',
                  queries: [{ refId: 'A', expr: `{mcp_server="${serverName}"}` }],
                  range: { from: 'now-1h', to: 'now' },
                }),
              )}`}
              target="_blank"
              rel="noopener noreferrer"
              onClick={(e) => e.stopPropagation()}
              className="text-muted-foreground hover:text-foreground"
              title="View in Grafana"
            >
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          )}
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </div>
      </button>
      {expanded && (
        <div className="border-t px-3 pb-3 pt-2 space-y-1">
          {toolsQuery.isLoading && (
            <div className="space-y-2">
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-full" />
            </div>
          )}
          {toolsQuery.isError && <p className="text-xs text-destructive">Failed to load tools.</p>}
          {tools.length === 0 && !toolsQuery.isLoading && !toolsQuery.isError && (
            <p className="text-xs text-muted-foreground">No tools available.</p>
          )}
          {tools.map((tool) => (
            <div key={tool.name} className="rounded border bg-background">
              <button
                type="button"
                className="flex w-full items-center justify-between px-3 py-2 text-left text-sm hover:bg-accent/30"
                onClick={() => setExpandedTool((prev) => (prev === tool.name ? null : tool.name))}
              >
                <div className="min-w-0">
                  <p className="font-mono text-xs font-medium">{tool.name}</p>
                  {tool.description && (
                    <p className="truncate text-xs text-muted-foreground">{tool.description}</p>
                  )}
                </div>
                {expandedTool === tool.name ? (
                  <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                ) : (
                  <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                )}
              </button>
              {expandedTool === tool.name && (
                <div className="border-t px-3 py-2">
                  <p className="mb-1 text-[10px] font-semibold text-muted-foreground uppercase">
                    Input Schema
                  </p>
                  <pre className="max-h-48 overflow-auto rounded bg-muted p-2 text-[11px] font-mono">
                    {JSON.stringify(tool.input_schema, null, 2)}
                  </pre>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Resources list ──────────────────────────────────────────

function ResourcesList({ serverName }: { serverName: string }) {
  const [expanded, setExpanded] = useState(false);

  const resourcesQuery = useQuery({
    queryKey: ['mcp-server-resources', serverName],
    queryFn: () => api.get<McpResourceView[]>(`/api/v1/mcp/servers/${serverName}/resources`),
    enabled: expanded,
  });

  const resources = resourcesQuery.data ?? [];

  return (
    <div className="rounded-lg border bg-muted/30">
      <button
        type="button"
        className="flex w-full items-center justify-between p-3 text-left transition-colors hover:bg-accent/50"
        onClick={() => setExpanded((v) => !v)}
      >
        <div className="flex items-center gap-2">
          <FileText className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
            Resources
          </span>
          {expanded && !resourcesQuery.isLoading && (
            <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
              {resources.length}
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-1">
          {expanded && (
            <a
              href={`/grafana/explore?orgId=1&left=${encodeURIComponent(
                JSON.stringify({
                  datasource: 'Quickwit',
                  queries: [{ refId: 'A', expr: `{mcp_server="${serverName}"}` }],
                  range: { from: 'now-1h', to: 'now' },
                }),
              )}`}
              target="_blank"
              rel="noopener noreferrer"
              onClick={(e) => e.stopPropagation()}
              className="text-muted-foreground hover:text-foreground"
              title="View in Grafana"
            >
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          )}
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </div>
      </button>
      {expanded && (
        <div className="border-t px-3 pb-3 pt-2 space-y-1">
          {resourcesQuery.isLoading && (
            <div className="space-y-2">
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-full" />
            </div>
          )}
          {resourcesQuery.isError && (
            <p className="text-xs text-destructive">Failed to load resources.</p>
          )}
          {resources.length === 0 && !resourcesQuery.isLoading && !resourcesQuery.isError && (
            <p className="text-xs text-muted-foreground">No resources available.</p>
          )}
          {resources.map((resource) => (
            <div key={resource.uri} className="rounded border bg-background px-3 py-2">
              <p className="font-mono text-xs font-medium">{resource.uri}</p>
              {resource.name && <p className="text-xs text-muted-foreground">{resource.name}</p>}
              {resource.description && (
                <p className="text-xs text-muted-foreground">{resource.description}</p>
              )}
              {resource.mime_type && (
                <Badge variant="outline" className="mt-1 text-[10px] px-1.5 py-0">
                  {resource.mime_type}
                </Badge>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Logs list ────────────────────────────────────────────────

function LogsList({ serverName }: { serverName: string }) {
  const [expanded, setExpanded] = useState(false);

  const logsQuery = useQuery({
    queryKey: ['mcp-server-logs', serverName],
    queryFn: () => api.get<McpLogEntry[]>(`/api/v1/mcp/servers/${serverName}/logs`),
    enabled: expanded,
    refetchInterval: expanded ? 3000 : false,
  });

  const logs = logsQuery.data ?? [];

  return (
    <div className="rounded-lg border bg-muted/30">
      <button
        type="button"
        className="flex w-full items-center justify-between p-3 text-left transition-colors hover:bg-accent/50"
        onClick={() => setExpanded((v) => !v)}
      >
        <div className="flex items-center gap-2">
          <ScrollText className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
            Logs
          </span>
          {expanded && !logsQuery.isLoading && logs.length > 0 && (
            <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
              {logs.length}
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-1">
          {expanded && (
            <a
              href={`/grafana/explore?orgId=1&left=${encodeURIComponent(
                JSON.stringify({
                  datasource: 'Quickwit',
                  queries: [{ refId: 'A', expr: `{mcp_server="${serverName}"}` }],
                  range: { from: 'now-1h', to: 'now' },
                }),
              )}`}
              target="_blank"
              rel="noopener noreferrer"
              onClick={(e) => e.stopPropagation()}
              className="text-muted-foreground hover:text-foreground"
              title="View in Grafana"
            >
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          )}
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </div>
      </button>
      {expanded && (
        <div className="border-t px-3 pb-3 pt-2">
          {logsQuery.isLoading && (
            <div className="space-y-2">
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-full" />
            </div>
          )}
          {logsQuery.isError && <p className="text-xs text-destructive">Failed to load logs.</p>}
          {logs.length === 0 && !logsQuery.isLoading && !logsQuery.isError && (
            <p className="text-xs text-muted-foreground">No logs yet.</p>
          )}
          {logs.length > 0 && (
            <div className="max-h-64 overflow-y-auto rounded border bg-muted/30 p-2 font-mono text-xs space-y-0.5">
              {logs.map((entry, i) => (
                <div key={i} className="flex gap-2 py-0.5">
                  <span className="shrink-0 text-muted-foreground">
                    {new Date(entry.timestamp).toLocaleTimeString()}
                  </span>
                  <Badge
                    variant={
                      entry.level === 'error'
                        ? 'destructive'
                        : entry.level === 'warn'
                          ? 'secondary'
                          : 'outline'
                    }
                    className="shrink-0 text-[10px] px-1 py-0"
                  >
                    {entry.level}
                  </Badge>
                  <span className={entry.level === 'error' ? 'text-destructive' : ''}>
                    {entry.message}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
