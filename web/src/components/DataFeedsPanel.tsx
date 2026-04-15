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

import { useState, useCallback } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  dataFeedsApi,
  type DataFeedConfig,
  type FeedEvent,
  type CreateFeedRequest,
  type AuthType,
} from "@/api/data-feeds";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { JsonTree } from "@/components/JsonTree";
import {
  AlertTriangle,
  ArrowLeft,
  ChevronRight,
  Clock,
  Copy,
  Pencil,
  Plus,
  Radio,
  Trash2,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Convert an ISO timestamp to a human-readable relative time string. */
function timeAgo(dateStr: string): string {
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  const diffSec = Math.floor((now - then) / 1000);

  if (diffSec < 0) return "just now";
  if (diffSec < 60) return `${diffSec}s ago`;

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;

  const diffHour = Math.floor(diffMin / 60);
  if (diffHour < 24) return `${diffHour}h ago`;

  const diffDay = Math.floor(diffHour / 24);
  return `${diffDay}d ago`;
}

/** Estimate byte-size of a JSON payload. */
function payloadSize(payload: unknown): string {
  const bytes = new Blob([JSON.stringify(payload)]).size;
  if (bytes < 1024) return `${bytes}B`;
  return `${(bytes / 1024).toFixed(1)}K`;
}

/** Format type badge label. */
function typeLabel(t: DataFeedConfig["feed_type"]): string {
  switch (t) {
    case "polling":
      return "Polling";
    case "webhook":
      return "Webhook";
    case "websocket":
      return "WebSocket";
  }
}

// ---------------------------------------------------------------------------
// Status badge
// ---------------------------------------------------------------------------

function StatusBadge({
  status,
  enabled,
}: {
  status: DataFeedConfig["status"];
  enabled: boolean;
}) {
  if (!enabled) {
    return (
      <Badge variant="secondary" className="text-muted-foreground">
        Disabled
      </Badge>
    );
  }
  switch (status) {
    case "running":
      return (
        <Badge className="border-green-200 bg-green-50 text-green-700 dark:border-green-900 dark:bg-green-950 dark:text-green-400">
          Running
        </Badge>
      );
    case "idle":
      return (
        <Badge className="border-yellow-200 bg-yellow-50 text-yellow-700 dark:border-yellow-900 dark:bg-yellow-950 dark:text-yellow-400">
          Idle
        </Badge>
      );
    case "error":
      return (
        <Badge variant="destructive">
          Error
        </Badge>
      );
  }
}

// ---------------------------------------------------------------------------
// Time filter options
// ---------------------------------------------------------------------------

const TIME_FILTERS = [
  { value: "1h", label: "Last 1 hour" },
  { value: "24h", label: "Last 24 hours" },
  { value: "7d", label: "Last 7 days" },
  { value: "30d", label: "Last 30 days" },
] as const;



// ---------------------------------------------------------------------------
// Empty auth/transport helpers
// ---------------------------------------------------------------------------

function emptyTransport(
  feedType: CreateFeedRequest["feed_type"],
): Record<string, unknown> {
  switch (feedType) {
    case "polling":
      return { url: "", interval_secs: 60, headers: {}, method: "GET" };
    case "webhook":
      return { events: [], body_size_limit: 1048576 };
    case "websocket":
      return {
        url: "",
        reconnect_backoff: [5, 10, 30, 60],
        heartbeat_secs: 30,
      };
  }
}

// ---------------------------------------------------------------------------
// Feed Form Dialog
// ---------------------------------------------------------------------------

interface FeedFormState {
  name: string;
  feed_type: CreateFeedRequest["feed_type"];
  tags: string;
  transport: Record<string, unknown>;
  authType: "none" | AuthType;
  authFields: Record<string, string>;
}

const INITIAL_FORM: FeedFormState = {
  name: "",
  feed_type: "polling",
  tags: "",
  transport: emptyTransport("polling"),
  authType: "none",
  authFields: {},
};

function feedToForm(feed: DataFeedConfig): FeedFormState {
  const authType: FeedFormState["authType"] = feed.auth
    ? (feed.auth.type as AuthType)
    : "none";
  const authFields: Record<string, string> = {};
  if (feed.auth) {
    for (const [k, v] of Object.entries(feed.auth)) {
      if (k !== "type") authFields[k] = String(v ?? "");
    }
  }
  return {
    name: feed.name,
    feed_type: feed.feed_type,
    tags: feed.tags.join(", "),
    transport: { ...feed.transport },
    authType,
    authFields,
  };
}

function formToRequest(form: FeedFormState): CreateFeedRequest {
  const tags = form.tags
    .split(",")
    .map((t) => t.trim())
    .filter(Boolean);

  let auth = null;
  if (form.authType !== "none") {
    auth = { type: form.authType, ...form.authFields };
  }

  return {
    name: form.name,
    feed_type: form.feed_type,
    tags,
    transport: form.transport,
    auth,
  };
}

function TransportFields({
  feedType,
  transport,
  onChange,
}: {
  feedType: CreateFeedRequest["feed_type"];
  transport: Record<string, unknown>;
  onChange: (t: Record<string, unknown>) => void;
}) {
  const set = (key: string, value: unknown) =>
    onChange({ ...transport, [key]: value });

  switch (feedType) {
    case "polling":
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">URL</Label>
            <Input
              value={String(transport.url ?? "")}
              onChange={(e) => set("url", e.target.value)}
              placeholder="https://api.example.com/data"
              className="h-9 font-mono text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Interval (seconds)</Label>
            <Input
              type="number"
              min={1}
              value={String(transport.interval_secs ?? 60)}
              onChange={(e) => set("interval_secs", Number(e.target.value))}
              className="h-9 w-32 text-sm"
            />
          </div>
        </div>
      );
    case "webhook":
      return (
        <p className="text-sm text-muted-foreground">
          A unique webhook URL will be generated after creation. Configure your
          external service to POST events to that URL.
        </p>
      );
    case "websocket":
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">WebSocket URL</Label>
            <Input
              value={String(transport.url ?? "")}
              onChange={(e) => set("url", e.target.value)}
              placeholder="wss://stream.example.com/ws"
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
  }
}

function AuthFields({
  authType,
  fields,
  onChange,
}: {
  authType: AuthType;
  fields: Record<string, string>;
  onChange: (f: Record<string, string>) => void;
}) {
  const set = (key: string, value: string) =>
    onChange({ ...fields, [key]: value });

  switch (authType) {
    case "header":
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Header Name</Label>
            <Input
              value={fields.name ?? ""}
              onChange={(e) => set("name", e.target.value)}
              placeholder="X-API-Key"
              className="h-9 text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Header Value</Label>
            <Input
              type="password"
              value={fields.value ?? ""}
              onChange={(e) => set("value", e.target.value)}
              placeholder="sk-..."
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
    case "query":
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Query Parameter</Label>
            <Input
              value={fields.name ?? ""}
              onChange={(e) => set("name", e.target.value)}
              placeholder="apikey"
              className="h-9 text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Value</Label>
            <Input
              type="password"
              value={fields.value ?? ""}
              onChange={(e) => set("value", e.target.value)}
              placeholder="sk-..."
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
    case "bearer":
      return (
        <div className="space-y-1.5">
          <Label className="text-sm font-medium">Bearer Token</Label>
          <Input
            type="password"
            value={fields.token ?? ""}
            onChange={(e) => set("token", e.target.value)}
            placeholder="eyJ..."
            className="h-9 font-mono text-sm"
          />
        </div>
      );
    case "basic":
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Username</Label>
            <Input
              value={fields.username ?? ""}
              onChange={(e) => set("username", e.target.value)}
              className="h-9 text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Password</Label>
            <Input
              type="password"
              value={fields.password ?? ""}
              onChange={(e) => set("password", e.target.value)}
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
    case "hmac":
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">HMAC Secret</Label>
            <Input
              type="password"
              value={fields.secret ?? ""}
              onChange={(e) => set("secret", e.target.value)}
              placeholder="whsec_..."
              className="h-9 font-mono text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Signature Header</Label>
            <Input
              value={fields.header ?? ""}
              onChange={(e) => set("header", e.target.value)}
              placeholder="X-Hub-Signature-256"
              className="h-9 text-sm"
            />
          </div>
        </div>
      );
  }
}

function FeedFormDialog({
  open,
  onOpenChange,
  editFeed,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  editFeed?: DataFeedConfig;
}) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<FeedFormState>(
    editFeed ? feedToForm(editFeed) : INITIAL_FORM,
  );
  const [error, setError] = useState<string | null>(null);

  const isEdit = !!editFeed;

  const createMutation = useMutation({
    mutationFn: (req: CreateFeedRequest) => dataFeedsApi.create(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-feeds"] });
      onOpenChange(false);
    },
    onError: (err: Error) => setError(err.message),
  });

  const updateMutation = useMutation({
    mutationFn: (req: Partial<CreateFeedRequest>) =>
      dataFeedsApi.update(editFeed!.id, req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-feeds"] });
      onOpenChange(false);
    },
    onError: (err: Error) => setError(err.message),
  });

  const handleSubmit = () => {
    setError(null);
    const req = formToRequest(form);
    if (!req.name.trim()) {
      setError("Name is required");
      return;
    }
    if (isEdit) {
      updateMutation.mutate(req);
    } else {
      createMutation.mutate(req);
    }
  };

  const saving = createMutation.isPending || updateMutation.isPending;

  // Reset form when dialog opens with a new feed
  const handleOpenChange = (next: boolean) => {
    if (next) {
      setForm(editFeed ? feedToForm(editFeed) : INITIAL_FORM);
      setError(null);
    }
    onOpenChange(next);
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{isEdit ? "Edit Feed" : "New Data Feed"}</DialogTitle>
          <DialogDescription>
            {isEdit
              ? "Update the data feed configuration."
              : "Configure an external data source to ingest events."}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          {/* Name */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Name</Label>
            <Input
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="e.g. github-rara"
              className="h-9 font-mono text-sm"
              disabled={isEdit}
            />
          </div>

          {/* Feed Type */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Type</Label>
            <Select
              value={form.feed_type}
              onValueChange={(v) => {
                const ft = v as CreateFeedRequest["feed_type"];
                setForm({
                  ...form,
                  feed_type: ft,
                  transport: emptyTransport(ft),
                });
              }}
              disabled={isEdit}
            >
              <SelectTrigger className="h-9 w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="polling">Polling</SelectItem>
                <SelectItem value="webhook">Webhook</SelectItem>
                <SelectItem value="websocket">WebSocket</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {/* Transport */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Transport</Label>
            <TransportFields
              feedType={form.feed_type}
              transport={form.transport}
              onChange={(t) => setForm({ ...form, transport: t })}
            />
          </div>

          {/* Auth */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Authentication</Label>
            <Select
              value={form.authType}
              onValueChange={(v) => {
                setForm({
                  ...form,
                  authType: v as FeedFormState["authType"],
                  authFields: {},
                });
              }}
            >
              <SelectTrigger className="h-9 w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">None</SelectItem>
                <SelectItem value="header">API Key (Header)</SelectItem>
                <SelectItem value="query">API Key (Query)</SelectItem>
                <SelectItem value="bearer">Bearer Token</SelectItem>
                <SelectItem value="basic">Basic Auth</SelectItem>
                <SelectItem value="hmac">HMAC Signature</SelectItem>
              </SelectContent>
            </Select>
            {form.authType !== "none" && (
              <div className="mt-3">
                <AuthFields
                  authType={form.authType}
                  fields={form.authFields}
                  onChange={(f) => setForm({ ...form, authFields: f })}
                />
              </div>
            )}
          </div>

          {/* Tags */}
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Tags</Label>
            <Input
              value={form.tags}
              onChange={(e) => setForm({ ...form, tags: e.target.value })}
              placeholder="stock, yahoo, aapl"
              className="h-9 text-sm"
            />
            <p className="text-xs text-muted-foreground">
              Comma-separated. Used for subscription matching.
            </p>
          </div>

          {/* Error */}
          {error && (
            <div className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              {error}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={saving}
          >
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={saving}>
            {saving ? "Saving..." : isEdit ? "Update" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Event Detail Sheet
// ---------------------------------------------------------------------------

function EventDetailSheet({
  event,
  open,
  onOpenChange,
}: {
  event: FeedEvent | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    if (!event) return;
    navigator.clipboard
      .writeText(JSON.stringify(event.payload, null, 2))
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      });
  }, [event]);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="overflow-y-auto sm:max-w-lg">
        <SheetHeader>
          <SheetTitle className="font-mono text-sm">
            {event?.id ?? "Event Detail"}
          </SheetTitle>
          <SheetDescription>
            {event ? timeAgo(event.received_at) : ""}
            {event && (
              <span
                className="ml-2 text-xs text-muted-foreground"
                title={event.received_at}
              >
                ({new Date(event.received_at).toLocaleString()})
              </span>
            )}
          </SheetDescription>
        </SheetHeader>

        {event && (
          <div className="mt-6 space-y-4">
            {/* Meta */}
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <span className="text-xs font-medium text-muted-foreground">
                  Type
                </span>
                <Badge variant="outline">{event.event_type}</Badge>
              </div>
              {event.tags.length > 0 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-xs font-medium text-muted-foreground">
                    Tags
                  </span>
                  {event.tags.map((tag) => (
                    <Badge key={tag} variant="secondary" className="text-xs">
                      {tag}
                    </Badge>
                  ))}
                </div>
              )}
            </div>

            {/* Payload */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium text-muted-foreground">
                  Payload
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1 text-xs"
                  onClick={handleCopy}
                >
                  <Copy className="h-3 w-3" />
                  {copied ? "Copied" : "Copy"}
                </Button>
              </div>
              <div className="rounded-lg border bg-muted/30 p-3 font-mono text-xs">
                <JsonTree data={event.payload} />
              </div>
            </div>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}

// ---------------------------------------------------------------------------
// Event History View
// ---------------------------------------------------------------------------

function EventHistoryView({
  feed,
  onBack,
}: {
  feed: DataFeedConfig;
  onBack: () => void;
}) {
  const [timeFilter, setTimeFilter] = useState("24h");
  const [selectedEvent, setSelectedEvent] = useState<FeedEvent | null>(null);
  const [offset, setOffset] = useState(0);
  const limit = 50;

  const since = timeFilter;

  const eventsQuery = useQuery({
    queryKey: ["data-feed-events", feed.id, since, offset],
    queryFn: () => dataFeedsApi.events(feed.id, { since, limit, offset }),
  });

  const events = eventsQuery.data?.events ?? [];
  const hasMore = eventsQuery.data?.has_more ?? false;
  const total = eventsQuery.data?.total ?? 0;

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center gap-3">
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-1"
          onClick={onBack}
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          Back
        </Button>
      </div>

      {/* Feed info card */}
      <div className="rounded-lg border bg-muted/20 px-4 py-3">
        <div className="flex items-center gap-3">
          <Radio className="h-5 w-5 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="font-semibold">{feed.name}</span>
              <Badge variant="outline" className="text-xs">
                {typeLabel(feed.feed_type)}
              </Badge>
              <StatusBadge status={feed.status} enabled={feed.enabled} />
            </div>
            <div className="mt-0.5 flex items-center gap-3 text-xs text-muted-foreground">
              {feed.feed_type === "polling" && !!feed.transport.url && (
                <span className="truncate font-mono">
                  {String(feed.transport.url)}
                </span>
              )}
              {feed.feed_type === "polling" && !!feed.transport.interval_secs && (
                <span>{String(feed.transport.interval_secs)}s interval</span>
              )}
              <span>{total} events</span>
            </div>
          </div>
        </div>
        {feed.last_error && (
          <div className="mt-2 flex items-center gap-2 text-xs text-destructive">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            {feed.last_error}
          </div>
        )}
      </div>

      {/* Filters */}
      <div className="flex items-center gap-3">
        <Select
          value={timeFilter}
          onValueChange={(v) => {
            setTimeFilter(v);
            setOffset(0);
          }}
        >
          <SelectTrigger className="h-8 w-44 text-xs">
            <Clock className="mr-1.5 h-3.5 w-3.5" />
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {TIME_FILTERS.map((f) => (
              <SelectItem key={f.value} value={f.value}>
                {f.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Events table */}
      {eventsQuery.isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={i} className="h-10 w-full" />
          ))}
        </div>
      ) : events.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
          <Clock className="mb-2 h-8 w-8" />
          <p className="text-sm">No events in this time range</p>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-32">Time</TableHead>
              <TableHead>Type</TableHead>
              <TableHead className="w-20 text-right">Size</TableHead>
              <TableHead className="w-8" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {events.map((evt) => (
              <TableRow
                key={evt.id}
                className="cursor-pointer"
                onClick={() => setSelectedEvent(evt)}
              >
                <TableCell
                  className="font-mono text-xs"
                  title={new Date(evt.received_at).toLocaleString()}
                >
                  {timeAgo(evt.received_at)}
                </TableCell>
                <TableCell>
                  <Badge variant="outline" className="text-xs">
                    {evt.event_type}
                  </Badge>
                </TableCell>
                <TableCell className="text-right text-xs text-muted-foreground">
                  {payloadSize(evt.payload)}
                </TableCell>
                <TableCell>
                  <ChevronRight className="h-4 w-4 text-muted-foreground" />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {/* Load more */}
      {hasMore && (
        <div className="flex justify-center">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setOffset((prev) => prev + limit)}
            disabled={eventsQuery.isFetching}
          >
            {eventsQuery.isFetching ? "Loading..." : "Load more"}
          </Button>
        </div>
      )}

      {/* Event detail drawer */}
      <EventDetailSheet
        event={selectedEvent}
        open={!!selectedEvent}
        onOpenChange={(open) => {
          if (!open) setSelectedEvent(null);
        }}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Feed List View
// ---------------------------------------------------------------------------

function FeedListView({
  feeds,
  onSelectFeed,
}: {
  feeds: DataFeedConfig[];
  onSelectFeed: (feed: DataFeedConfig) => void;
}) {
  const queryClient = useQueryClient();
  const [formOpen, setFormOpen] = useState(false);
  const [editFeed, setEditFeed] = useState<DataFeedConfig | undefined>();
  const [deleteId, setDeleteId] = useState<string | null>(null);

  const toggleMutation = useMutation({
    mutationFn: (id: string) => dataFeedsApi.toggle(id),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["data-feeds"] }),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => dataFeedsApi.delete(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-feeds"] });
      setDeleteId(null);
    },
  });

  const handleEdit = (feed: DataFeedConfig) => {
    setEditFeed(feed);
    setFormOpen(true);
  };

  const handleNew = () => {
    setEditFeed(undefined);
    setFormOpen(true);
  };

  return (
    <>
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-base font-semibold">Data Feeds</h2>
          <p className="text-xs text-muted-foreground">
            External data sources that push events into rara.
          </p>
        </div>
        <Button size="sm" className="h-8 gap-1" onClick={handleNew}>
          <Plus className="h-3.5 w-3.5" />
          New Feed
        </Button>
      </div>

      {/* Table */}
      {feeds.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed py-12 text-muted-foreground">
          <Radio className="mb-2 h-8 w-8" />
          <p className="text-sm">No data feeds configured</p>
          <p className="text-xs">
            Create a feed to start ingesting external events.
          </p>
          <Button
            size="sm"
            variant="outline"
            className="mt-4 gap-1"
            onClick={handleNew}
          >
            <Plus className="h-3.5 w-3.5" />
            Create Feed
          </Button>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead className="w-24">Type</TableHead>
              <TableHead className="w-24">Status</TableHead>
              <TableHead className="w-24 text-right">Updated</TableHead>
              <TableHead className="w-32 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {feeds.map((feed) => (
              <TableRow key={feed.id}>
                <TableCell>
                  <button
                    className="font-medium text-primary hover:underline"
                    onClick={() => onSelectFeed(feed)}
                  >
                    {feed.name}
                  </button>
                  {feed.tags.length > 0 && (
                    <div className="mt-0.5 flex flex-wrap gap-1">
                      {feed.tags.slice(0, 3).map((tag) => (
                        <Badge
                          key={tag}
                          variant="secondary"
                          className="text-[10px] px-1.5 py-0"
                        >
                          {tag}
                        </Badge>
                      ))}
                      {feed.tags.length > 3 && (
                        <span className="text-[10px] text-muted-foreground">
                          +{feed.tags.length - 3}
                        </span>
                      )}
                    </div>
                  )}
                </TableCell>
                <TableCell>
                  <Badge variant="outline" className="text-xs">
                    {typeLabel(feed.feed_type)}
                  </Badge>
                </TableCell>
                <TableCell>
                  <StatusBadge status={feed.status} enabled={feed.enabled} />
                </TableCell>
                <TableCell
                  className="text-right text-xs text-muted-foreground"
                  title={new Date(feed.updated_at).toLocaleString()}
                >
                  {timeAgo(feed.updated_at)}
                </TableCell>
                <TableCell className="text-right">
                  <div className="flex items-center justify-end gap-2">
                    <Switch
                      checked={feed.enabled}
                      onCheckedChange={() => toggleMutation.mutate(feed.id)}
                      disabled={toggleMutation.isPending}
                    />
                    <Button
                      variant="outline"
                      size="icon"
                      className="h-7 w-7"
                      onClick={() => handleEdit(feed)}
                    >
                      <Pencil className="h-3 w-3" />
                    </Button>
                    <Button
                      variant="outline"
                      size="icon"
                      className="h-7 w-7 text-destructive hover:bg-destructive/10"
                      onClick={() => setDeleteId(feed.id)}
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {/* Create/Edit Dialog */}
      <FeedFormDialog
        open={formOpen}
        onOpenChange={setFormOpen}
        editFeed={editFeed}
      />

      {/* Delete Confirmation */}
      <Dialog
        open={!!deleteId}
        onOpenChange={(open) => {
          if (!open) setDeleteId(null);
        }}
      >
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>Delete Feed</DialogTitle>
            <DialogDescription>
              This will permanently remove this feed and stop all event
              ingestion. This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteId(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => deleteId && deleteMutation.mutate(deleteId)}
              disabled={deleteMutation.isPending}
            >
              {deleteMutation.isPending ? "Deleting..." : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

// ---------------------------------------------------------------------------
// Root Component
// ---------------------------------------------------------------------------

type View =
  | { kind: "list" }
  | { kind: "events"; feed: DataFeedConfig };

export default function DataFeedsPanel() {
  const [view, setView] = useState<View>({ kind: "list" });

  const feedsQuery = useQuery({
    queryKey: ["data-feeds"],
    queryFn: () => dataFeedsApi.list(),
  });

  if (feedsQuery.isLoading) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-8 w-48" />
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={i} className="h-14 w-full" />
        ))}
      </div>
    );
  }

  if (feedsQuery.isError) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
        <AlertTriangle className="mb-2 h-8 w-8 text-destructive" />
        <p className="text-sm">Failed to load data feeds</p>
        <p className="text-xs">Check the backend connection and try again.</p>
        <Button
          variant="outline"
          size="sm"
          className="mt-3"
          onClick={() => feedsQuery.refetch()}
        >
          Retry
        </Button>
      </div>
    );
  }

  const feeds = feedsQuery.data ?? [];

  if (view.kind === "events") {
    // When we navigate to events, refresh the feed object from the list
    // so toggling status is reflected.
    const freshFeed =
      feeds.find((f) => f.id === view.feed.id) ?? view.feed;
    return (
      <EventHistoryView
        feed={freshFeed}
        onBack={() => setView({ kind: "list" })}
      />
    );
  }

  return (
    <FeedListView
      feeds={feeds}
      onSelectFeed={(feed) => setView({ kind: "events", feed })}
    />
  );
}
