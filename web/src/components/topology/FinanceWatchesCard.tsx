/*
 * Copyright 2026 Rararulab
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

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';

import {
  CANDLE_STREAM_OVERVIEW_LIMIT,
  dataFeedsApi,
  type CandleStream,
  type CreateFinanceSubscriptionRequest,
  type FeedCatalogEntry,
  type FinanceEventKind,
  type FinanceSubscription,
} from '@/api/data-feeds';
import { useSettingsModal } from '@/components/settings/SettingsModalContext';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { expectedCandleStreamHealthForGroups } from '@/lib/finance-candle-health';

interface FinanceWatchesCardProps {
  sessionKey: string;
}

export function FinanceWatchesCard({ sessionKey }: FinanceWatchesCardProps) {
  const queryClient = useQueryClient();
  const { openSettings } = useSettingsModal();
  const catalogQuery = useQuery({
    queryKey: ['data-feed-catalog'],
    queryFn: () => dataFeedsApi.catalog(),
    staleTime: 30_000,
  });
  const subscriptionsQuery = useQuery({
    queryKey: ['finance-subscriptions'],
    queryFn: () => dataFeedsApi.financeSubscriptions(),
    staleTime: 30_000,
  });
  const candleStreamsQuery = useQuery({
    queryKey: ['market-data-candle-streams'],
    queryFn: () => dataFeedsApi.candleStreams({ limit: CANDLE_STREAM_OVERVIEW_LIMIT }),
    refetchInterval: 30_000,
    staleTime: 30_000,
  });

  const subscribeMutation = useMutation({
    mutationFn: (entry: FeedCatalogEntry) =>
      dataFeedsApi.createFinanceSubscription(subscriptionRequestForEntry(entry, sessionKey)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['market-data-candle-streams'] });
    },
  });
  const enableMutation = useMutation({
    mutationFn: (entry: FeedCatalogEntry) => dataFeedsApi.enableCatalogEntry(entry.id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feeds'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-summaries'] });
      void queryClient.invalidateQueries({ queryKey: ['market-data-candle-streams'] });
    },
  });
  const unsubscribeMutation = useMutation({
    mutationFn: ({
      entry,
      subscriptionIds,
    }: {
      entry: FeedCatalogEntry;
      subscriptionIds: string[];
    }) => dataFeedsApi.unsubscribeCatalogEntry(entry.id, { subscription_ids: subscriptionIds }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['market-data-candle-streams'] });
    },
  });

  const entries = (catalogQuery.data ?? []).filter(isFinanceWatchableEntry);
  const candleStreams = candleStreamsQuery.data?.streams ?? [];
  const candleStreamLimitReached =
    candleStreamsQuery.data != null &&
    candleStreamsQuery.data.streams.length >= candleStreamsQuery.data.query_limit;
  const sessionSubscriptions = (subscriptionsQuery.data?.subscriptions ?? []).filter(
    (subscription) => subscription.session_key === sessionKey,
  );
  const loading = catalogQuery.isLoading || subscriptionsQuery.isLoading;
  const error =
    catalogQuery.error?.message ??
    subscriptionsQuery.error?.message ??
    candleStreamsQuery.error?.message ??
    enableMutation.error?.message ??
    subscribeMutation.error?.message ??
    unsubscribeMutation.error?.message ??
    null;

  return (
    <Card className="rounded-lg">
      <CardHeader className="p-3 pb-2">
        <div className="flex items-center justify-between gap-2">
          <div>
            <CardTitle className="text-xs">Finance watches</CardTitle>
            <CardDescription className="mt-1 text-[11px]">
              Subscribe this session to built-in news and K-line feeds.
            </CardDescription>
          </div>
          {sessionSubscriptions.length > 0 && (
            <Badge variant="outline" className="shrink-0 text-[10px]">
              {sessionSubscriptions.length} active
            </Badge>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-2 p-3 pt-0">
        {loading ? (
          <div className="rounded-md border border-dashed px-3 py-2 text-[11px] text-muted-foreground">
            Loading finance sources…
          </div>
        ) : entries.length === 0 ? (
          <div className="rounded-md border border-dashed px-3 py-2 text-[11px] text-muted-foreground">
            No finance feed sources are configured.
          </div>
        ) : (
          <div className="space-y-2">
            {candleStreamLimitReached && (
              <div className="rounded-md border border-dashed bg-muted/30 px-3 py-2 text-[11px] text-muted-foreground">
                K-line stream overview is limited to {candleStreamsQuery.data.query_limit} streams;
                a missing health check may need a narrower source, symbol, or timeframe query.
              </div>
            )}
            {entries.map((entry) => {
              const matchingSubscriptions = sessionSubscriptions.filter((subscription) =>
                subscriptionMatchesEntry(subscription, entry),
              );
              const subscribed = matchingSubscriptions.length > 0;
              const canEnableInline = !entry.enabled && !entry.requires_configuration;
              const needsConfiguration = !entry.enabled && entry.requires_configuration;
              const streamHealth =
                subscribed && entry.feed_type === 'market_candle'
                  ? marketCandleEntryStreamHealth(
                      entry,
                      matchingSubscriptions,
                      candleStreams,
                      candleStreamsQuery.isLoading,
                    )
                  : null;
              const pending =
                (enableMutation.isPending && enableMutation.variables?.id === entry.id) ||
                (subscribeMutation.isPending && subscribeMutation.variables?.id === entry.id) ||
                (unsubscribeMutation.isPending &&
                  unsubscribeMutation.variables?.entry.id === entry.id);
              const actionLabel = financeWatchActionLabel({
                pending,
                canEnableInline,
                needsConfiguration,
                subscribed,
              });
              return (
                <div key={entry.id} className="rounded-md border bg-background/60 px-3 py-2">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0 space-y-1">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="truncate text-xs font-medium">{entry.name}</span>
                        <Badge variant="outline" className="px-1.5 py-0 text-[10px]">
                          {entry.feed_type === 'rss' ? 'News' : 'K-line'}
                        </Badge>
                        {!entry.enabled && (
                          <Badge variant="secondary" className="px-1.5 py-0 text-[10px]">
                            {needsConfiguration ? 'needs config' : 'source off'}
                          </Badge>
                        )}
                        {subscribed && (
                          <Badge variant="secondary" className="px-1.5 py-0 text-[10px]">
                            watching
                          </Badge>
                        )}
                      </div>
                      <p className="line-clamp-2 text-[11px] text-muted-foreground">
                        {coverageLabel(entry)}
                      </p>
                      {streamHealth && (
                        <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
                          <Badge
                            variant={streamHealth.status === 'missing' ? 'secondary' : 'outline'}
                            className={
                              streamHealth.status === 'fresh'
                                ? 'px-1.5 py-0 text-[10px] text-foreground'
                                : streamHealth.status === 'stale'
                                  ? 'px-1.5 py-0 text-[10px] text-amber-600'
                                  : 'px-1.5 py-0 text-[10px] text-muted-foreground'
                            }
                          >
                            Data {streamHealth.label}
                          </Badge>
                          <span>{streamHealth.detail}</span>
                        </div>
                      )}
                    </div>
                    <Button
                      size="sm"
                      variant={subscribed ? 'outline' : 'default'}
                      className="h-7 shrink-0 px-2 text-[11px]"
                      disabled={pending}
                      onClick={() => {
                        if (canEnableInline) {
                          enableMutation.mutate(entry);
                        } else if (needsConfiguration) {
                          openSettings('data-feeds', { dataFeedCatalogId: entry.id });
                        } else if (subscribed) {
                          unsubscribeMutation.mutate({
                            entry,
                            subscriptionIds: matchingSubscriptions.map(
                              (subscription) => subscription.subscription_id,
                            ),
                          });
                        } else {
                          subscribeMutation.mutate(entry);
                        }
                      }}
                    >
                      {actionLabel}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
        {error && (
          <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-[11px] text-destructive">
            {error}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function isFinanceWatchableEntry(entry: FeedCatalogEntry): boolean {
  return entry.feed_type === 'rss' || entry.feed_type === 'market_candle';
}

function financeWatchActionLabel({
  pending,
  canEnableInline,
  needsConfiguration,
  subscribed,
}: {
  pending: boolean;
  canEnableInline: boolean;
  needsConfiguration: boolean;
  subscribed: boolean;
}): string {
  if (pending) {
    if (canEnableInline) return 'Enabling…';
    return subscribed ? 'Removing…' : 'Adding…';
  }
  if (needsConfiguration) return 'Configure';
  if (canEnableInline) return 'Enable source';
  return subscribed ? 'Unwatch' : 'Watch';
}

function subscriptionRequestForEntry(
  entry: FeedCatalogEntry,
  sessionKey: string,
): CreateFinanceSubscriptionRequest {
  const request: CreateFinanceSubscriptionRequest = {
    session_key: sessionKey,
    catalog_source_ids: [entry.id],
    delivery: 'silent',
  };
  if (entry.feed_type === 'market_candle') {
    request.venues = optionalList(catalogVenue(entry));
    request.symbols = catalogSymbols(entry);
    request.timeframes = catalogTimeframes(entry);
  }
  return request;
}

function subscriptionMatchesEntry(
  subscription: FinanceSubscription,
  entry: FeedCatalogEntry,
): boolean {
  return (
    subscription.source_names.includes(catalogSourceName(entry)) &&
    subscription.event_kinds.includes(eventKindForEntry(entry))
  );
}

function eventKindForEntry(entry: FeedCatalogEntry): FinanceEventKind {
  return entry.feed_type === 'market_candle' ? 'market_candle_closed' : 'rss_article';
}

function catalogSourceName(entry: FeedCatalogEntry): string {
  return entry.source_name?.trim() || `finance-${entry.id}`;
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

function coverageLabel(entry: FeedCatalogEntry): string {
  if (entry.feed_type === 'market_candle') {
    return [
      catalogVenue(entry),
      summarizeList(catalogSymbols(entry)),
      summarizeList(catalogTimeframes(entry), 2),
    ]
      .filter(Boolean)
      .join(' · ');
  }

  const tags = entry.tags.filter((tag) => tag !== 'finance' && tag !== 'news');
  return tags.length > 0 ? tags.join(' · ') : entry.description;
}

function marketCandleEntryStreamHealth(
  entry: FeedCatalogEntry,
  subscriptions: FinanceSubscription[],
  streams: CandleStream[],
  loading: boolean,
): {
  status: 'checking' | 'fresh' | 'stale' | 'missing';
  label: string;
  detail: string;
} {
  if (loading) {
    return {
      status: 'checking',
      label: 'Checking',
      detail: 'Checking stored K-line streams.',
    };
  }

  return expectedCandleStreamHealthForGroups(
    subscriptions.map((subscription) => ({
      sourceName: catalogSourceName(entry),
      venue: subscription.venues[0] ?? catalogVenue(entry),
      symbols: subscription.symbols.length > 0 ? subscription.symbols : catalogSymbols(entry),
      timeframes:
        subscription.timeframes.length > 0 ? subscription.timeframes : catalogTimeframes(entry),
    })),
    streams,
  );
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

function optionalList(value: string | null): string[] {
  return value ? [value] : [];
}

function summarizeList(values: string[], max = 3): string {
  if (values.length <= max) return values.join(', ');
  return `${values.slice(0, max).join(', ')} +${values.length - max}`;
}
