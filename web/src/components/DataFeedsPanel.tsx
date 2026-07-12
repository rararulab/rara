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
} from 'lucide-react';
import { useState, useCallback, useEffect } from 'react';

import {
  dataFeedsApi,
  type CandleStream,
  type DataFeedConfig,
  type FeedCatalogEntry,
  type FeedEvent,
  type FeedSummary,
  type FinanceSubscription,
  type FinanceSubscriptionsResponse,
  type CreateFeedRequest,
  type EnableCatalogEntryRequest,
  type AuthType,
} from '@/api/data-feeds';
import { JsonTree } from '@/components/JsonTree';
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
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet';
import { Skeleton } from '@/components/ui/skeleton';
import { Switch } from '@/components/ui/switch';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Convert an ISO timestamp to a human-readable relative time string. */
function timeAgo(dateStr: string): string {
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  const diffSec = Math.floor((now - then) / 1000);

  if (diffSec < 0) return 'just now';
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

function eventCountLabel(count: number): string {
  return `${count} event${count === 1 ? '' : 's'}`;
}

function lagLabel(summary: FeedSummary | undefined): string {
  if (!summary?.last_event_at || summary.lag_seconds == null) return 'No events yet';
  if (summary.lag_seconds < 60) return `${summary.lag_seconds}s lag`;

  const minutes = Math.floor(summary.lag_seconds / 60);
  if (minutes < 60) return `${minutes}m lag`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h lag`;

  return `${Math.floor(hours / 24)}d lag`;
}

/** Format type badge label. */
function typeLabel(t: DataFeedConfig['feed_type']): string {
  switch (t) {
    case 'polling':
      return 'Polling';
    case 'webhook':
      return 'Webhook';
    case 'websocket':
      return 'WebSocket';
    case 'rss':
      return 'RSS';
    case 'market_candle':
      return 'Market Candle';
  }
}

function transportString(entry: FeedCatalogEntry, key: string): string | null {
  const value = entry.transport_template?.[key];
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function transportStringList(entry: FeedCatalogEntry, key: string): string[] {
  const value = entry.transport_template?.[key];
  if (!Array.isArray(value)) return [];
  return value
    .filter((item): item is string => typeof item === 'string')
    .map((item) => item.trim())
    .filter(Boolean);
}

function catalogVenue(entry: FeedCatalogEntry): string | null {
  return entry.venue?.trim() || transportString(entry, 'venue');
}

function catalogSymbols(entry: FeedCatalogEntry): string[] {
  return entry.configured_symbols?.length
    ? entry.configured_symbols
    : transportStringList(entry, 'symbols');
}

function catalogTimeframes(entry: FeedCatalogEntry): string[] {
  return entry.configured_timeframes?.length
    ? entry.configured_timeframes
    : transportStringList(entry, 'timeframes');
}

function summarizeList(values: string[], max = 4): string {
  if (values.length <= max) return values.join(', ');
  return `${values.slice(0, max).join(', ')} +${values.length - max}`;
}

function catalogCoverageLabel(entry: FeedCatalogEntry): string | null {
  if (entry.feed_type === 'market_candle') {
    const parts = [
      catalogVenue(entry),
      summarizeList(catalogSymbols(entry)),
      summarizeList(catalogTimeframes(entry), 3),
    ].filter(Boolean);
    return parts.length > 0 ? parts.join(' · ') : null;
  }

  if (entry.feed_type === 'rss') {
    const tags = entry.tags.filter((tag) => tag !== 'finance' && tag !== 'news');
    return tags.length > 0 ? `RSS news · ${tags.join(' · ')}` : 'RSS news';
  }

  return null;
}

function catalogSourceName(entry: FeedCatalogEntry): string {
  return entry.source_name?.trim() || `finance-${entry.id}`;
}

function financeEventKindLabel(kind: FinanceSubscription['event_kinds'][number]): string {
  switch (kind) {
    case 'rss_article':
      return 'News';
    case 'market_candle_closed':
      return 'K-line';
  }
}

function financeSubscriptionTitle(subscription: FinanceSubscription): string {
  const source =
    subscription.sources[0]?.catalog_name ??
    subscription.source_names[0] ??
    (subscription.matches_all_sources ? 'All finance sources' : 'Custom source');
  const kinds = subscription.event_kinds.map(financeEventKindLabel).join(' + ');
  return kinds ? `${source} · ${kinds}` : source;
}

function financeSubscriptionSelectors(subscription: FinanceSubscription): string[] {
  const selectors = [
    summarizeList(subscription.symbols),
    summarizeList(subscription.timeframes, 3),
    summarizeList(subscription.category_tags),
    summarizeList(subscription.watch_terms),
  ].filter(Boolean);
  return selectors.length > 0 ? selectors : ['All matching events'];
}

function streamLabel(stream: CandleStream): string {
  return `${stream.venue.toUpperCase()} · ${stream.symbol} · ${stream.timeframe}`;
}

type CandleSubscriptionStreamHealth = {
  status: 'fresh' | 'stale' | 'missing';
  label: string;
  detail: string;
};

function normalizedSelectorSet(
  values: string[],
  mode: 'lower' | 'upper' | 'timeframe',
): Set<string> {
  return new Set(
    values
      .map((value) => value.trim())
      .filter(Boolean)
      .map((value) => {
        if (mode === 'lower') return value.toLowerCase();
        if (mode === 'upper') return value.toUpperCase();
        return value.toLowerCase();
      }),
  );
}

function streamMatchesSubscription(
  stream: CandleStream,
  subscription: FinanceSubscription,
): boolean {
  const sourceNames = normalizedSelectorSet(subscription.source_names, 'lower');
  const venues = normalizedSelectorSet(subscription.venues, 'lower');
  const symbols = normalizedSelectorSet(subscription.symbols, 'upper');
  const timeframes = normalizedSelectorSet(subscription.timeframes, 'timeframe');

  if (
    sourceNames.size > 0 &&
    !subscription.matches_all_sources &&
    !sourceNames.has(stream.source_name.toLowerCase())
  ) {
    return false;
  }
  if (venues.size > 0 && !venues.has(stream.venue.toLowerCase())) return false;
  if (symbols.size > 0 && !symbols.has(stream.symbol.toUpperCase())) return false;
  if (timeframes.size > 0 && !timeframes.has(stream.timeframe.toLowerCase())) return false;
  return true;
}

function timeframeSeconds(timeframe: string): number | null {
  const match = /^(\d+)([smhd])$/i.exec(timeframe.trim());
  if (!match) return null;
  const [, amountText, unit] = match;
  if (!amountText || !unit) return null;
  const amount = Number(amountText);
  if (!Number.isFinite(amount) || amount <= 0) return null;
  switch (unit.toLowerCase()) {
    case 's':
      return amount;
    case 'm':
      return amount * 60;
    case 'h':
      return amount * 60 * 60;
    case 'd':
      return amount * 24 * 60 * 60;
    default:
      return null;
  }
}

function isStaleStream(stream: CandleStream): boolean {
  const stepSeconds = timeframeSeconds(stream.timeframe);
  if (stepSeconds == null) return false;
  const latestCloseMs = new Date(stream.latest_close_time).getTime();
  if (!Number.isFinite(latestCloseMs)) return false;
  const lagSeconds = Math.floor((Date.now() - latestCloseMs) / 1000);
  return lagSeconds > stepSeconds * 2;
}

function candleSubscriptionStreamHealth(
  subscription: FinanceSubscription,
  streams: CandleStream[],
): CandleSubscriptionStreamHealth | null {
  if (!subscription.event_kinds.includes('market_candle_closed')) return null;

  const matchingStreams = streams.filter((stream) =>
    streamMatchesSubscription(stream, subscription),
  );
  if (matchingStreams.length === 0) {
    return {
      status: 'missing',
      label: 'Missing',
      detail: 'No stored K-line stream matches this subscription yet.',
    };
  }

  const staleCount = matchingStreams.filter(isStaleStream).length;
  if (staleCount === matchingStreams.length) {
    return {
      status: 'stale',
      label: 'Stale',
      detail: `${staleCount} matched stream${staleCount === 1 ? '' : 's'} past the freshness window.`,
    };
  }

  const freshCount = matchingStreams.length - staleCount;
  return {
    status: 'fresh',
    label: 'Fresh',
    detail: `${freshCount}/${matchingStreams.length} matched stream${matchingStreams.length === 1 ? '' : 's'} fresh.`,
  };
}

// ---------------------------------------------------------------------------
// Status badge
// ---------------------------------------------------------------------------

function StatusBadge({ status, enabled }: { status: DataFeedConfig['status']; enabled: boolean }) {
  if (!enabled) {
    return (
      <Badge variant="secondary" className="text-muted-foreground">
        Disabled
      </Badge>
    );
  }
  switch (status) {
    case 'running':
      return (
        <Badge variant="outline" className="gap-1.5 text-foreground">
          <span className="h-1.5 w-1.5 rounded-full bg-primary" aria-hidden="true" />
          Running
        </Badge>
      );
    case 'idle':
      return (
        <Badge variant="outline" className="gap-1.5 text-muted-foreground">
          <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground" aria-hidden="true" />
          Idle
        </Badge>
      );
    case 'error':
      return <Badge variant="destructive">Error</Badge>;
  }
}

// ---------------------------------------------------------------------------
// Time filter options
// ---------------------------------------------------------------------------

const TIME_FILTERS = [
  { value: '1h', label: 'Last 1 hour' },
  { value: '24h', label: 'Last 24 hours' },
  { value: '7d', label: 'Last 7 days' },
  { value: '30d', label: 'Last 30 days' },
] as const;

// ---------------------------------------------------------------------------
// Empty auth/transport helpers
// ---------------------------------------------------------------------------

function emptyTransport(feedType: CreateFeedRequest['feed_type']): Record<string, unknown> {
  switch (feedType) {
    case 'polling':
      return { url: '', interval_secs: 60, headers: {}, method: 'GET' };
    case 'webhook':
      return { events: [], body_size_limit: 1048576 };
    case 'websocket':
      return {
        url: '',
        reconnect_backoff: [5, 10, 30, 60],
        heartbeat_secs: 30,
      };
    case 'rss':
      return { url: '', interval_secs: 300, headers: {}, max_entries_per_poll: 50 };
    case 'market_candle':
      return {
        url: '',
        interval_secs: 60,
        headers: {},
        venue: '',
        symbols: [],
        timeframes: [],
        max_candles_per_poll: 1000,
      };
  }
}

// ---------------------------------------------------------------------------
// Feed Form Dialog
// ---------------------------------------------------------------------------

interface FeedFormState {
  name: string;
  feed_type: CreateFeedRequest['feed_type'];
  tags: string;
  transport: Record<string, unknown>;
  authType: 'none' | AuthType;
  authFields: Record<string, string>;
}

const INITIAL_FORM: FeedFormState = {
  name: '',
  feed_type: 'polling',
  tags: '',
  transport: emptyTransport('polling'),
  authType: 'none',
  authFields: {},
};

function feedToForm(feed: DataFeedConfig): FeedFormState {
  const authType: FeedFormState['authType'] = feed.auth ? feed.auth.type : 'none';
  const authFields: Record<string, string> = {};
  if (feed.auth) {
    for (const [k, v] of Object.entries(feed.auth)) {
      if (k !== 'type') authFields[k] = String(v ?? '');
    }
  }
  return {
    name: feed.name,
    feed_type: feed.feed_type,
    tags: feed.tags.join(', '),
    transport: { ...feed.transport },
    authType,
    authFields,
  };
}

function formToRequest(form: FeedFormState): CreateFeedRequest {
  const tags = form.tags
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);

  let auth = null;
  if (form.authType !== 'none') {
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
  feedType: CreateFeedRequest['feed_type'];
  transport: Record<string, unknown>;
  onChange: (t: Record<string, unknown>) => void;
}) {
  const set = (key: string, value: unknown) => onChange({ ...transport, [key]: value });

  switch (feedType) {
    case 'polling':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">URL</Label>
            <Input
              value={String(transport.url ?? '')}
              onChange={(e) => set('url', e.target.value)}
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
              onChange={(e) => set('interval_secs', Number(e.target.value))}
              className="h-9 w-32 text-sm"
            />
          </div>
        </div>
      );
    case 'webhook':
      return (
        <p className="text-sm text-muted-foreground">
          A unique webhook URL will be generated after creation. Configure your external service to
          POST events to that URL.
        </p>
      );
    case 'websocket':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">WebSocket URL</Label>
            <Input
              value={String(transport.url ?? '')}
              onChange={(e) => set('url', e.target.value)}
              placeholder="wss://stream.example.com/ws"
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
    case 'rss':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">RSS/Atom URL</Label>
            <Input
              value={String(transport.url ?? '')}
              onChange={(e) => set('url', e.target.value)}
              placeholder="https://example.com/feed.xml"
              className="h-9 font-mono text-sm"
            />
          </div>
          <div className="flex gap-3">
            <div className="space-y-1.5">
              <Label className="text-sm font-medium">Interval (seconds)</Label>
              <Input
                type="number"
                min={1}
                value={String(transport.interval_secs ?? 300)}
                onChange={(e) => set('interval_secs', Number(e.target.value))}
                className="h-9 w-32 text-sm"
              />
            </div>
            <div className="space-y-1.5">
              <Label className="text-sm font-medium">Max entries</Label>
              <Input
                type="number"
                min={1}
                value={String(transport.max_entries_per_poll ?? 50)}
                onChange={(e) => set('max_entries_per_poll', Number(e.target.value))}
                className="h-9 w-32 text-sm"
              />
            </div>
          </div>
        </div>
      );
    case 'market_candle':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Normalized candle endpoint</Label>
            <Input
              value={String(transport.url ?? '')}
              onChange={(e) => set('url', e.target.value)}
              placeholder="https://market-data.example/candles/latest"
              className="h-9 font-mono text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Venue</Label>
            <Input
              value={String(transport.venue ?? '')}
              onChange={(e) => set('venue', e.target.value)}
              placeholder="binance"
              className="h-9 text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Symbols</Label>
            <Input
              value={Array.isArray(transport.symbols) ? transport.symbols.join(', ') : ''}
              onChange={(e) =>
                set(
                  'symbols',
                  e.target.value
                    .split(',')
                    .map((item) => item.trim())
                    .filter(Boolean),
                )
              }
              placeholder="BTCUSDT, ETHUSDT"
              className="h-9 font-mono text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Timeframes</Label>
            <Input
              value={Array.isArray(transport.timeframes) ? transport.timeframes.join(', ') : ''}
              onChange={(e) =>
                set(
                  'timeframes',
                  e.target.value
                    .split(',')
                    .map((item) => item.trim())
                    .filter(Boolean),
                )
              }
              placeholder="1m, 15m, 1h"
              className="h-9 font-mono text-sm"
            />
            <p className="text-xs text-muted-foreground">
              Endpoint must return rara's normalized candle batch JSON.
            </p>
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
  const set = (key: string, value: string) => onChange({ ...fields, [key]: value });

  switch (authType) {
    case 'header':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Header Name</Label>
            <Input
              value={fields.name ?? ''}
              onChange={(e) => set('name', e.target.value)}
              placeholder="X-API-Key"
              className="h-9 text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Header Value</Label>
            <Input
              type="password"
              value={fields.value ?? ''}
              onChange={(e) => set('value', e.target.value)}
              placeholder="sk-..."
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
    case 'query':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Query Parameter</Label>
            <Input
              value={fields.name ?? ''}
              onChange={(e) => set('name', e.target.value)}
              placeholder="apikey"
              className="h-9 text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Value</Label>
            <Input
              type="password"
              value={fields.value ?? ''}
              onChange={(e) => set('value', e.target.value)}
              placeholder="sk-..."
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
    case 'bearer':
      return (
        <div className="space-y-1.5">
          <Label className="text-sm font-medium">Bearer Token</Label>
          <Input
            type="password"
            value={fields.token ?? ''}
            onChange={(e) => set('token', e.target.value)}
            placeholder="eyJ..."
            className="h-9 font-mono text-sm"
          />
        </div>
      );
    case 'basic':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Username</Label>
            <Input
              value={fields.username ?? ''}
              onChange={(e) => set('username', e.target.value)}
              className="h-9 text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Password</Label>
            <Input
              type="password"
              value={fields.password ?? ''}
              onChange={(e) => set('password', e.target.value)}
              className="h-9 font-mono text-sm"
            />
          </div>
        </div>
      );
    case 'hmac':
      return (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">HMAC Secret</Label>
            <Input
              type="password"
              value={fields.secret ?? ''}
              onChange={(e) => set('secret', e.target.value)}
              placeholder="whsec_..."
              className="h-9 font-mono text-sm"
            />
          </div>
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Signature Header</Label>
            <Input
              value={fields.header ?? ''}
              onChange={(e) => set('header', e.target.value)}
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
  initialForm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  editFeed?: DataFeedConfig | undefined;
  initialForm?: FeedFormState | undefined;
}) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<FeedFormState>(
    editFeed ? feedToForm(editFeed) : (initialForm ?? INITIAL_FORM),
  );
  const [error, setError] = useState<string | null>(null);

  const isEdit = !!editFeed;

  const createMutation = useMutation({
    mutationFn: (req: CreateFeedRequest) => dataFeedsApi.create(req),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['data-feeds'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-summaries'] });
      onOpenChange(false);
    },
    onError: (err: Error) => setError(err.message),
  });

  const updateMutation = useMutation({
    mutationFn: (req: Partial<CreateFeedRequest>) => dataFeedsApi.update(editFeed!.id, req),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['data-feeds'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-summaries'] });
      onOpenChange(false);
    },
    onError: (err: Error) => setError(err.message),
  });

  const handleSubmit = () => {
    setError(null);
    const req = formToRequest(form);
    if (!req.name.trim()) {
      setError('Name is required');
      return;
    }
    if (isEdit) {
      updateMutation.mutate(req);
    } else {
      createMutation.mutate(req);
    }
  };

  const saving = createMutation.isPending || updateMutation.isPending;

  useEffect(() => {
    if (!open) return;
    setForm(editFeed ? feedToForm(editFeed) : (initialForm ?? INITIAL_FORM));
    setError(null);
  }, [open, editFeed, initialForm]);

  // Reset form when dialog opens with a new feed
  const handleOpenChange = (next: boolean) => {
    if (next) {
      setForm(editFeed ? feedToForm(editFeed) : (initialForm ?? INITIAL_FORM));
      setError(null);
    }
    onOpenChange(next);
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{isEdit ? 'Edit Feed' : 'New Data Feed'}</DialogTitle>
          <DialogDescription>
            {isEdit
              ? 'Update the data feed configuration.'
              : 'Configure an external data source to ingest events.'}
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
                const ft = v as CreateFeedRequest['feed_type'];
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
                <SelectItem value="rss">RSS</SelectItem>
                <SelectItem value="market_candle">Market Candle</SelectItem>
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
                  authType: v as FeedFormState['authType'],
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
            {form.authType !== 'none' && (
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
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={saving}>
            {saving ? 'Saving...' : isEdit ? 'Update' : 'Create'}
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
    void navigator.clipboard.writeText(JSON.stringify(event.payload, null, 2)).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [event]);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="overflow-y-auto sm:max-w-lg">
        <SheetHeader>
          <SheetTitle className="font-mono text-sm">{event?.id ?? 'Event Detail'}</SheetTitle>
          <SheetDescription>
            {event ? timeAgo(event.received_at) : ''}
            {event && (
              <span className="ml-2 text-xs text-muted-foreground" title={event.received_at}>
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
                <span className="text-xs font-medium text-muted-foreground">Type</span>
                <Badge variant="outline">{event.event_type}</Badge>
              </div>
              {event.tags.length > 0 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-xs font-medium text-muted-foreground">Tags</span>
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
                <span className="text-xs font-medium text-muted-foreground">Payload</span>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1 text-xs"
                  onClick={handleCopy}
                >
                  <Copy className="h-3 w-3" />
                  {copied ? 'Copied' : 'Copy'}
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

function EventHistoryView({ feed, onBack }: { feed: DataFeedConfig; onBack: () => void }) {
  const [timeFilter, setTimeFilter] = useState('24h');
  const [selectedEvent, setSelectedEvent] = useState<FeedEvent | null>(null);
  const [offset, setOffset] = useState(0);
  const limit = 50;

  const since = timeFilter;

  const eventsQuery = useQuery({
    queryKey: ['data-feed-events', feed.id, since, offset],
    queryFn: () => dataFeedsApi.events(feed.id, { since, limit, offset }),
  });

  const events = eventsQuery.data?.events ?? [];
  const hasMore = eventsQuery.data?.has_more ?? false;
  const total = eventsQuery.data?.total ?? 0;

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center gap-3">
        <Button variant="outline" size="sm" className="h-8 gap-1" onClick={onBack}>
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
              {feed.feed_type === 'polling' && !!feed.transport.url && (
                <span className="truncate font-mono">{String(feed.transport.url)}</span>
              )}
              {feed.feed_type === 'polling' && !!feed.transport.interval_secs && (
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
            {eventsQuery.isFetching ? 'Loading...' : 'Load more'}
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

function catalogEntryToForm(entry: FeedCatalogEntry): FeedFormState {
  return {
    name: `finance-${entry.id}`,
    feed_type: entry.feed_type,
    tags: entry.tags.join(', '),
    transport: entry.transport_template ?? emptyTransport(entry.feed_type),
    authType: 'none',
    authFields: {},
  };
}

function buildCatalogEnableRequest(form: FeedFormState): EnableCatalogEntryRequest {
  const req = formToRequest(form);
  return {
    transport: req.transport,
    auth: req.auth,
  };
}

function validateCatalogEnableForm(form: FeedFormState): string | null {
  if (form.feed_type !== 'market_candle') return null;

  const provider = String(form.transport.provider ?? 'normalized');
  const url = String(form.transport.url ?? '').trim();
  if (provider === 'normalized' && !url) return 'Normalized candle endpoint is required';

  if (!Array.isArray(form.transport.symbols) || form.transport.symbols.length === 0) {
    return 'At least one symbol is required';
  }

  if (!Array.isArray(form.transport.timeframes) || form.transport.timeframes.length === 0) {
    return 'At least one timeframe is required';
  }

  return null;
}

function CatalogConfigureDialog({
  entry,
  onOpenChange,
  onSubmit,
  saving,
  serverError,
}: {
  entry: FeedCatalogEntry | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (body: EnableCatalogEntryRequest) => void;
  saving: boolean;
  serverError: string | null;
}) {
  const [form, setForm] = useState<FeedFormState>(entry ? catalogEntryToForm(entry) : INITIAL_FORM);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!entry) return;
    setForm(catalogEntryToForm(entry));
    setError(null);
  }, [entry]);

  const handleSubmit = () => {
    const validationError = validateCatalogEnableForm(form);
    if (validationError) {
      setError(validationError);
      return;
    }
    setError(null);
    onSubmit(buildCatalogEnableRequest(form));
  };

  return (
    <Dialog open={!!entry} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{entry ? `Configure ${entry.name}` : 'Configure source'}</DialogTitle>
          <DialogDescription>
            Fill the operator-owned feed settings, then materialize this built-in source.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Transport</Label>
            <TransportFields
              feedType={form.feed_type}
              transport={form.transport}
              onChange={(transport) => setForm({ ...form, transport })}
            />
          </div>

          <div className="space-y-1.5">
            <Label className="text-sm font-medium">Authentication</Label>
            <Select
              value={form.authType}
              onValueChange={(v) => {
                setForm({
                  ...form,
                  authType: v as FeedFormState['authType'],
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
            {form.authType !== 'none' && (
              <div className="mt-3">
                <AuthFields
                  authType={form.authType}
                  fields={form.authFields}
                  onChange={(authFields) => setForm({ ...form, authFields })}
                />
              </div>
            )}
          </div>

          {(error || serverError) && (
            <div className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              {error ?? serverError}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={saving}>
            {saving ? 'Enabling...' : 'Enable source'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function FeedCatalogCard({
  entries,
  onUseTemplate,
  focusCatalogEntryId,
}: {
  entries: FeedCatalogEntry[];
  onUseTemplate: (entry: FeedCatalogEntry) => void;
  focusCatalogEntryId?: string | undefined;
}) {
  const queryClient = useQueryClient();
  const [configureEntry, setConfigureEntry] = useState<FeedCatalogEntry | null>(null);

  useEffect(() => {
    if (!focusCatalogEntryId) return;
    const entry = entries.find((candidate) => candidate.id === focusCatalogEntryId);
    if (entry?.requires_configuration) {
      setConfigureEntry(entry);
    }
  }, [entries, focusCatalogEntryId]);

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: ['data-feeds'] });
    void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
    void queryClient.invalidateQueries({ queryKey: ['data-feed-summaries'] });
    void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
  };

  const enableMutation = useMutation({
    mutationFn: ({ id, body }: { id: string; body?: EnableCatalogEntryRequest }) =>
      dataFeedsApi.enableCatalogEntry(id, body),
    onSuccess: refresh,
  });

  const disableMutation = useMutation({
    mutationFn: (id: string) => dataFeedsApi.disableCatalogEntry(id),
    onSuccess: refresh,
  });

  const unsubscribeMutation = useMutation({
    mutationFn: (id: string) => dataFeedsApi.unsubscribeCatalogEntry(id),
    onSuccess: refresh,
  });

  if (entries.length === 0) return null;

  const newsEntries = entries.filter((entry) => entry.feed_type === 'rss');
  const candleEntries = entries.filter(
    (entry) => entry.feed_type === 'market_candle' && !entry.requires_configuration,
  );
  const providerEntries = entries.filter((entry) => entry.requires_configuration);

  const renderEntry = (entry: FeedCatalogEntry) => {
    const pending =
      (enableMutation.isPending && enableMutation.variables?.id === entry.id) ||
      (disableMutation.isPending && disableMutation.variables === entry.id) ||
      (unsubscribeMutation.isPending && unsubscribeMutation.variables === entry.id);
    const coverage = catalogCoverageLabel(entry);
    const subscribed = entry.subscriptions?.user_subscribed === true;
    const unsubscribeButton = subscribed ? (
      <Button
        variant="outline"
        size="sm"
        className="h-8 shrink-0"
        onClick={() => unsubscribeMutation.mutate(entry.id)}
        disabled={pending}
      >
        {pending && unsubscribeMutation.variables === entry.id ? 'Unsubscribing...' : 'Unsubscribe'}
      </Button>
    ) : null;

    return (
      <div
        key={entry.id}
        className="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between"
      >
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">{entry.name}</span>
            <Badge variant="outline" className="text-xs">
              {typeLabel(entry.feed_type)}
            </Badge>
            {entry.enabled && (
              <Badge variant="secondary" className="text-xs text-foreground">
                Enabled
              </Badge>
            )}
            {subscribed && (
              <Badge variant="outline" className="text-xs text-foreground">
                Subscribed
              </Badge>
            )}
            {entry.requires_configuration && (
              <Badge variant="secondary" className="text-xs text-muted-foreground">
                Requires config
              </Badge>
            )}
          </div>
          <p className="text-xs text-muted-foreground">{entry.description}</p>
          <p className="font-mono text-[11px] text-muted-foreground">
            Source {catalogSourceName(entry)}
          </p>
          {entry.setup_hint && <p className="text-xs text-muted-foreground">{entry.setup_hint}</p>}
          {coverage && <p className="text-[11px] text-muted-foreground">{coverage}</p>}
        </div>
        {entry.requires_configuration ? (
          <div className="flex shrink-0 gap-2">
            {unsubscribeButton}
            <Button
              size="sm"
              className="h-8"
              onClick={() => setConfigureEntry(entry)}
              disabled={pending}
            >
              Configure
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-8"
              onClick={() => onUseTemplate(entry)}
            >
              Use template
            </Button>
          </div>
        ) : entry.enabled ? (
          <div className="flex shrink-0 gap-2">
            {unsubscribeButton}
            <Button
              variant="outline"
              size="sm"
              className="h-8"
              onClick={() => disableMutation.mutate(entry.id)}
              disabled={pending}
            >
              {pending && disableMutation.variables === entry.id ? 'Disabling...' : 'Disable'}
            </Button>
          </div>
        ) : (
          <div className="flex shrink-0 gap-2">
            {unsubscribeButton}
            <Button
              size="sm"
              className="h-8"
              onClick={() => enableMutation.mutate({ id: entry.id })}
              disabled={pending}
            >
              {pending && enableMutation.variables?.id === entry.id ? 'Enabling...' : 'Enable'}
            </Button>
          </div>
        )}
      </div>
    );
  };

  return (
    <>
      <div className="rounded-lg border bg-card">
        <div className="border-b px-4 py-3">
          <h3 className="text-sm font-semibold">Default finance sources</h3>
          <p className="text-xs text-muted-foreground">
            Enable operator-owned news and K-line ingestion, then subscribe conversations to the
            sources they should watch.
          </p>
        </div>
        <div className="divide-y">
          {newsEntries.length > 0 && (
            <div>
              <div className="px-4 pt-3 text-xs font-medium text-muted-foreground">News feeds</div>
              {newsEntries.map(renderEntry)}
            </div>
          )}
          {candleEntries.length > 0 && (
            <div>
              <div className="px-4 pt-3 text-xs font-medium text-muted-foreground">
                K-line feeds
              </div>
              {candleEntries.map(renderEntry)}
            </div>
          )}
          {providerEntries.length > 0 && (
            <div>
              <div className="px-4 pt-3 text-xs font-medium text-muted-foreground">
                Provider presets
              </div>
              {providerEntries.map(renderEntry)}
            </div>
          )}
        </div>
      </div>
      <CatalogConfigureDialog
        entry={configureEntry}
        onOpenChange={(open) => {
          if (!open) setConfigureEntry(null);
        }}
        onSubmit={(body) => {
          if (!configureEntry) return;
          enableMutation.mutate(
            { id: configureEntry.id, body },
            {
              onSuccess: () => setConfigureEntry(null),
            },
          );
        }}
        saving={enableMutation.isPending && enableMutation.variables?.id === configureEntry?.id}
        serverError={
          enableMutation.isError && enableMutation.variables?.id === configureEntry?.id
            ? enableMutation.error.message
            : null
        }
      />
    </>
  );
}

function FinanceSubscriptionsCard({
  result,
  candleStreams,
}: {
  result: FinanceSubscriptionsResponse;
  candleStreams: CandleStream[];
}) {
  const queryClient = useQueryClient();
  const deleteMutation = useMutation({
    mutationFn: (id: string) => dataFeedsApi.deleteFinanceSubscription(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
    },
  });

  return (
    <div className="rounded-lg border bg-card">
      <div className="border-b px-4 py-3">
        <h3 className="text-sm font-semibold">Finance subscriptions</h3>
        <p className="text-xs text-muted-foreground">
          Conversation-created watches for finance news and K-line events. Feed ingestion keeps
          running when a subscription is removed.
        </p>
      </div>
      {result.count === 0 ? (
        <div className="px-4 py-4 text-xs text-muted-foreground">
          No active finance subscriptions for the current user.
        </div>
      ) : (
        <div className="divide-y">
          {result.subscriptions.map((subscription) => {
            const deleting =
              deleteMutation.isPending && deleteMutation.variables === subscription.subscription_id;
            const streamHealth = candleSubscriptionStreamHealth(subscription, candleStreams);
            return (
              <div
                key={subscription.subscription_id}
                className="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between"
              >
                <div className="min-w-0 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-medium">
                      {financeSubscriptionTitle(subscription)}
                    </span>
                    <Badge variant="outline" className="text-xs">
                      {subscription.delivery}
                    </Badge>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {financeSubscriptionSelectors(subscription).join(' · ')}
                  </p>
                  {streamHealth && (
                    <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      <Badge
                        variant={streamHealth.status === 'missing' ? 'secondary' : 'outline'}
                        className={
                          streamHealth.status === 'fresh'
                            ? 'text-foreground'
                            : streamHealth.status === 'stale'
                              ? 'text-amber-600'
                              : 'text-muted-foreground'
                        }
                      >
                        K-line {streamHealth.label}
                      </Badge>
                      <span>{streamHealth.detail}</span>
                    </div>
                  )}
                  <p className="font-mono text-[11px] text-muted-foreground">
                    {subscription.subscription_id}
                  </p>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 shrink-0 gap-1"
                  onClick={() => deleteMutation.mutate(subscription.subscription_id)}
                  disabled={deleting}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  {deleting ? 'Removing...' : 'Remove'}
                </Button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function MarketDataStreamsCard({
  streams,
  isLoading,
  isError,
  onRetry,
}: {
  streams: CandleStream[];
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
}) {
  return (
    <div className="rounded-lg border bg-card">
      <div className="flex items-center justify-between gap-3 border-b px-4 py-3">
        <div>
          <h3 className="text-sm font-semibold">Stored K-line streams</h3>
          <p className="text-xs text-muted-foreground">
            Latest closed candles persisted in the market-data repository.
          </p>
        </div>
        <Badge variant="outline" className="shrink-0 text-xs">
          {streams.length} stream{streams.length === 1 ? '' : 's'}
        </Badge>
      </div>
      {isLoading ? (
        <div className="space-y-2 px-4 py-4">
          <Skeleton className="h-4 w-1/2" />
          <Skeleton className="h-4 w-2/3" />
        </div>
      ) : isError ? (
        <div className="flex items-center justify-between gap-3 px-4 py-4">
          <p className="text-xs text-muted-foreground">Failed to load K-line stream watermarks.</p>
          <Button variant="outline" size="sm" className="h-8" onClick={onRetry}>
            Retry
          </Button>
        </div>
      ) : streams.length === 0 ? (
        <div className="px-4 py-4 text-xs text-muted-foreground">
          No closed candles have been stored yet. Enable a K-line feed and wait for the first
          persisted candle.
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Stream</TableHead>
              <TableHead className="w-36 text-right">Candles</TableHead>
              <TableHead className="w-40 text-right">Latest Close</TableHead>
              <TableHead className="w-40 text-right">Ingested</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {streams.map((stream) => (
              <TableRow
                key={`${stream.source_name}:${stream.venue}:${stream.symbol}:${stream.timeframe}`}
              >
                <TableCell>
                  <div className="space-y-0.5">
                    <div className="text-sm font-medium">{streamLabel(stream)}</div>
                    <div className="font-mono text-[11px] text-muted-foreground">
                      {stream.source_name}
                    </div>
                  </div>
                </TableCell>
                <TableCell className="text-right text-xs text-muted-foreground">
                  {stream.candle_count.toLocaleString()}
                </TableCell>
                <TableCell
                  className="text-right text-xs text-muted-foreground"
                  title={new Date(stream.latest_close_time).toLocaleString()}
                >
                  {timeAgo(stream.latest_close_time)}
                </TableCell>
                <TableCell
                  className="text-right text-xs text-muted-foreground"
                  title={new Date(stream.latest_ingested_at).toLocaleString()}
                >
                  {timeAgo(stream.latest_ingested_at)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}

function FeedListView({
  feeds,
  summaries,
  candleStreams,
  candleStreamsLoading,
  candleStreamsError,
  onRetryCandleStreams,
  onSelectFeed,
  focusCatalogEntryId,
}: {
  feeds: DataFeedConfig[];
  summaries: FeedSummary[];
  candleStreams: CandleStream[];
  candleStreamsLoading: boolean;
  candleStreamsError: boolean;
  onRetryCandleStreams: () => void;
  onSelectFeed: (feed: DataFeedConfig) => void;
  focusCatalogEntryId?: string | undefined;
}) {
  const queryClient = useQueryClient();
  const [formOpen, setFormOpen] = useState(false);
  const [editFeed, setEditFeed] = useState<DataFeedConfig | undefined>();
  const [draftForm, setDraftForm] = useState<FeedFormState | undefined>();
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const catalogQuery = useQuery({
    queryKey: ['data-feed-catalog'],
    queryFn: () => dataFeedsApi.catalog(),
  });
  const financeSubscriptionsQuery = useQuery({
    queryKey: ['finance-subscriptions'],
    queryFn: () => dataFeedsApi.financeSubscriptions(),
  });

  const toggleMutation = useMutation({
    mutationFn: (id: string) => dataFeedsApi.toggle(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['data-feeds'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => dataFeedsApi.delete(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['data-feeds'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-summaries'] });
      void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
      setDeleteId(null);
    },
  });

  const summariesByFeedId = new Map(summaries.map((summary) => [summary.feed_id, summary]));

  const handleEdit = (feed: DataFeedConfig) => {
    setEditFeed(feed);
    setDraftForm(undefined);
    setFormOpen(true);
  };

  const handleNew = () => {
    setEditFeed(undefined);
    setDraftForm(undefined);
    setFormOpen(true);
  };

  const handleUseTemplate = (entry: FeedCatalogEntry) => {
    setEditFeed(undefined);
    setDraftForm({
      name: `finance-${entry.id}`,
      feed_type: entry.feed_type,
      tags: entry.tags.join(', '),
      transport: entry.transport_template ?? emptyTransport(entry.feed_type),
      authType: 'none',
      authFields: {},
    });
    setFormOpen(true);
  };

  return (
    <div className="space-y-4">
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

      {catalogQuery.data && (
        <FeedCatalogCard
          entries={catalogQuery.data}
          onUseTemplate={handleUseTemplate}
          focusCatalogEntryId={focusCatalogEntryId}
        />
      )}
      {financeSubscriptionsQuery.data && (
        <FinanceSubscriptionsCard
          result={financeSubscriptionsQuery.data}
          candleStreams={candleStreams}
        />
      )}
      <MarketDataStreamsCard
        streams={candleStreams}
        isLoading={candleStreamsLoading}
        isError={candleStreamsError}
        onRetry={onRetryCandleStreams}
      />

      {/* Table */}
      {feeds.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed py-12 text-muted-foreground">
          <Radio className="mb-2 h-8 w-8" />
          <p className="text-sm">No data feeds configured</p>
          <p className="text-xs">Create a feed to start ingesting external events.</p>
          <Button size="sm" variant="outline" className="mt-4 gap-1" onClick={handleNew}>
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
              <TableHead className="w-24 text-right">Events</TableHead>
              <TableHead className="w-28 text-right">Last Event</TableHead>
              <TableHead className="w-24 text-right">Updated</TableHead>
              <TableHead className="w-32 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {feeds.map((feed) => {
              const summary = summariesByFeedId.get(feed.id);
              return (
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
                          <Badge key={tag} variant="secondary" className="text-[10px] px-1.5 py-0">
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
                  <TableCell className="text-right text-xs text-muted-foreground">
                    {eventCountLabel(summary?.event_count ?? 0)}
                  </TableCell>
                  <TableCell
                    className="text-right text-xs text-muted-foreground"
                    title={
                      summary?.last_event_at
                        ? new Date(summary.last_event_at).toLocaleString()
                        : undefined
                    }
                  >
                    {lagLabel(summary)}
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
              );
            })}
          </TableBody>
        </Table>
      )}

      {/* Create/Edit Dialog */}
      <FeedFormDialog
        open={formOpen}
        onOpenChange={setFormOpen}
        editFeed={editFeed}
        initialForm={draftForm}
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
              This will permanently remove this feed and stop all event ingestion. This action
              cannot be undone.
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
              {deleteMutation.isPending ? 'Deleting...' : 'Delete'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Root Component
// ---------------------------------------------------------------------------

type View = { kind: 'list' } | { kind: 'events'; feed: DataFeedConfig };

export default function DataFeedsPanel({
  focusCatalogEntryId,
}: {
  focusCatalogEntryId?: string | undefined;
} = {}) {
  const [view, setView] = useState<View>({ kind: 'list' });

  const feedsQuery = useQuery({
    queryKey: ['data-feeds'],
    queryFn: () => dataFeedsApi.list(),
  });
  const summariesQuery = useQuery({
    queryKey: ['data-feed-summaries'],
    queryFn: () => dataFeedsApi.summaries(),
    refetchInterval: 30_000,
  });
  const candleStreamsQuery = useQuery({
    queryKey: ['market-data-candle-streams'],
    queryFn: () => dataFeedsApi.candleStreams({ limit: 100 }),
    refetchInterval: 30_000,
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
        <Button variant="outline" size="sm" className="mt-3" onClick={() => feedsQuery.refetch()}>
          Retry
        </Button>
      </div>
    );
  }

  const feeds = feedsQuery.data ?? [];
  const summaries = summariesQuery.data ?? [];

  if (view.kind === 'events') {
    // When we navigate to events, refresh the feed object from the list
    // so toggling status is reflected.
    const freshFeed = feeds.find((f) => f.id === view.feed.id) ?? view.feed;
    return <EventHistoryView feed={freshFeed} onBack={() => setView({ kind: 'list' })} />;
  }

  return (
    <FeedListView
      feeds={feeds}
      summaries={summaries}
      candleStreams={candleStreamsQuery.data?.streams ?? []}
      candleStreamsLoading={candleStreamsQuery.isLoading}
      candleStreamsError={candleStreamsQuery.isError}
      onRetryCandleStreams={() => {
        void candleStreamsQuery.refetch();
      }}
      onSelectFeed={(feed) => setView({ kind: 'events', feed })}
      focusCatalogEntryId={focusCatalogEntryId}
    />
  );
}
