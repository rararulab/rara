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
  type FinanceFeedBundle,
  type FinanceFeedBundleQuickStartHint,
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
  const bundlesQuery = useQuery({
    queryKey: ['finance-feed-bundles'],
    queryFn: () => dataFeedsApi.financeBundles(),
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
  const subscribeBundleMutation = useMutation({
    mutationFn: (bundle: FinanceFeedBundle) =>
      dataFeedsApi.createFinanceSubscription(subscriptionRequestForBundle(bundle, sessionKey)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['finance-feed-bundles'] });
      void queryClient.invalidateQueries({ queryKey: ['market-data-candle-streams'] });
    },
  });
  const enableMutation = useMutation({
    mutationFn: (entry: FeedCatalogEntry) => dataFeedsApi.enableCatalogEntry(entry.id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['finance-feed-bundles'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feeds'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-summaries'] });
      void queryClient.invalidateQueries({ queryKey: ['market-data-candle-streams'] });
    },
  });
  const enableBundleMutation = useMutation({
    mutationFn: (bundle: FinanceFeedBundle) =>
      Promise.all(
        bundle.sources
          .filter((source) => !source.enabled && !source.requires_configuration)
          .map((source) => dataFeedsApi.enableCatalogEntry(source.id)),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['finance-feed-bundles'] });
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
      void queryClient.invalidateQueries({ queryKey: ['finance-feed-bundles'] });
      void queryClient.invalidateQueries({ queryKey: ['market-data-candle-streams'] });
    },
  });
  const unsubscribeBundleMutation = useMutation({
    mutationFn: (subscriptionIds: string[]) =>
      Promise.all(
        subscriptionIds.map((subscriptionId) =>
          dataFeedsApi.deleteFinanceSubscription(subscriptionId),
        ),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['finance-subscriptions'] });
      void queryClient.invalidateQueries({ queryKey: ['data-feed-catalog'] });
      void queryClient.invalidateQueries({ queryKey: ['finance-feed-bundles'] });
      void queryClient.invalidateQueries({ queryKey: ['market-data-candle-streams'] });
    },
  });

  const entries = (catalogQuery.data ?? []).filter(isFinanceWatchableEntry);
  const bundles = bundlesQuery.data?.bundles ?? [];
  const bundleQuickStartHints = new Map(
    (bundlesQuery.data?.quick_start_hints ?? []).map((hint) => [hint.bundle_id, hint]),
  );
  const candleStreams = candleStreamsQuery.data?.streams ?? [];
  const candleStreamHasMore = candleStreamsQuery.data?.has_more ?? false;
  const sessionSubscriptions = (subscriptionsQuery.data?.subscriptions ?? []).filter(
    (subscription) => subscription.session_key === sessionKey,
  );
  const loading = catalogQuery.isLoading || subscriptionsQuery.isLoading || bundlesQuery.isLoading;
  const error =
    catalogQuery.error?.message ??
    bundlesQuery.error?.message ??
    subscriptionsQuery.error?.message ??
    candleStreamsQuery.error?.message ??
    enableMutation.error?.message ??
    enableBundleMutation.error?.message ??
    subscribeMutation.error?.message ??
    subscribeBundleMutation.error?.message ??
    unsubscribeMutation.error?.message ??
    unsubscribeBundleMutation.error?.message ??
    null;

  return (
    <Card className="rounded-lg">
      <CardHeader className="p-3 pb-2">
        <div className="flex items-center justify-between gap-2">
          <div>
            <CardTitle className="text-xs">Finance watches</CardTitle>
            <CardDescription className="mt-1 text-[11px]">
              Enable starts background ingestion; Watch routes matching finance events into this
              session.
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
        ) : entries.length === 0 && bundles.length === 0 ? (
          <div className="rounded-md border border-dashed px-3 py-2 text-[11px] text-muted-foreground">
            No finance feed sources are configured.
          </div>
        ) : (
          <div className="space-y-2">
            {candleStreamHasMore && candleStreamsQuery.data != null && (
              <div className="rounded-md border border-dashed bg-muted/30 px-3 py-2 text-[11px] text-muted-foreground">
                K-line stream overview is limited to {candleStreamsQuery.data.query_limit} streams;
                a missing health check may need a narrower source, symbol, or timeframe query.
              </div>
            )}
            {bundles.length > 0 && (
              <div className="space-y-2">
                <div className="text-[11px] font-medium text-muted-foreground">Curated bundles</div>
                {bundles.map((bundle) => {
                  const quickStartHint = bundleQuickStartHints.get(bundle.id);
                  const matchingSubscriptions = sessionSubscriptions.filter((subscription) =>
                    subscriptionMatchesBundle(subscription, bundle),
                  );
                  const subscribed = matchingSubscriptions.length > 0;
                  const canEnableInline =
                    !subscribed &&
                    bundle.can_enable &&
                    bundle.sources.some((source) => !source.enabled);
                  const configurationSource = bundle.sources.find(
                    (source) => !source.enabled && source.requires_configuration,
                  );
                  const configurationSourceId =
                    quickStartConfigureSourceId(quickStartHint) ?? configurationSource?.id ?? null;
                  const needsConfiguration = !subscribed && configurationSourceId != null;
                  const quickStartAvailable = !subscribed && quickStartHint?.can_start_now === true;
                  const quickStartNeedsConfiguration =
                    !subscribed && quickStartHint?.requires_configuration === true;
                  const pending =
                    (enableBundleMutation.isPending &&
                      enableBundleMutation.variables?.id === bundle.id) ||
                    (subscribeBundleMutation.isPending &&
                      subscribeBundleMutation.variables?.id === bundle.id) ||
                    (unsubscribeBundleMutation.isPending &&
                      matchingSubscriptions.some((subscription) =>
                        unsubscribeBundleMutation.variables?.includes(subscription.subscription_id),
                      ));
                  const providers = summarizeList(bundle.providers, 2);
                  const load = bundleLoadLabel(bundle);
                  const fanoutDiagnostic = bundleFanoutDiagnostic(bundle);
                  return (
                    <div key={bundle.id} className="rounded-md border bg-muted/20 px-3 py-2">
                      <div className="flex items-start justify-between gap-2">
                        <div className="min-w-0 space-y-1">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <span className="truncate text-xs font-medium">{bundle.name}</span>
                            <Badge variant="outline" className="px-1.5 py-0 text-[10px]">
                              Default bundle
                            </Badge>
                            {providers && (
                              <Badge
                                variant="outline"
                                className="px-1.5 py-0 text-[10px] text-muted-foreground"
                              >
                                {providers}
                              </Badge>
                            )}
                            <Badge variant="secondary" className="px-1.5 py-0 text-[10px]">
                              {bundle.enabled_source_count}/{bundle.source_count} sources on
                            </Badge>
                            {subscribed && (
                              <Badge variant="secondary" className="px-1.5 py-0 text-[10px]">
                                watching
                              </Badge>
                            )}
                            {quickStartAvailable && (
                              <Badge variant="outline" className="px-1.5 py-0 text-[10px]">
                                Rara quick start
                              </Badge>
                            )}
                            {quickStartNeedsConfiguration && (
                              <Badge variant="secondary" className="px-1.5 py-0 text-[10px]">
                                setup required
                              </Badge>
                            )}
                            {fanoutDiagnostic && (
                              <Badge
                                variant="outline"
                                className="border-destructive/40 px-1.5 py-0 text-[10px] text-destructive"
                              >
                                unsafe fan-out
                              </Badge>
                            )}
                          </div>
                          <p className="line-clamp-2 text-[11px] text-muted-foreground">
                            {bundle.description}
                          </p>
                          <p className="text-[11px] text-muted-foreground">
                            {bundle.sources.map((source) => source.name).join(', ')}
                          </p>
                          <p className="text-[11px] text-muted-foreground">
                            {bundleLifecycleLabel({
                              bundle,
                              canEnableInline,
                              needsConfiguration,
                              quickStartAvailable,
                              subscribed,
                            })}
                          </p>
                          {load && <p className="text-[11px] text-muted-foreground">{load}</p>}
                          {fanoutDiagnostic && (
                            <p className="text-[11px] text-destructive">{fanoutDiagnostic}</p>
                          )}
                        </div>
                        <Button
                          size="sm"
                          variant={subscribed ? 'outline' : 'default'}
                          className="h-7 shrink-0 px-2 text-[11px]"
                          disabled={pending}
                          onClick={() => {
                            if (canEnableInline) {
                              enableBundleMutation.mutate(bundle);
                            } else if (needsConfiguration && configurationSourceId) {
                              openSettings('data-feeds', {
                                dataFeedCatalogId: configurationSourceId,
                              });
                            } else if (subscribed) {
                              unsubscribeBundleMutation.mutate(
                                matchingSubscriptions.map(
                                  (subscription) => subscription.subscription_id,
                                ),
                              );
                            } else {
                              subscribeBundleMutation.mutate(bundle);
                            }
                          }}
                        >
                          {financeBundleActionLabel({
                            pending,
                            canEnableInline,
                            needsConfiguration,
                            subscribed,
                          })}
                        </Button>
                      </div>
                    </div>
                  );
                })}
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
              const provider = catalogProvider(entry);
              const actionLabel = financeWatchActionLabel({
                pending,
                canEnableInline,
                needsConfiguration,
                subscribed,
              });
              const load = catalogLoadLabel(entry);
              const fanoutDiagnostic = catalogFanoutDiagnostic(entry);
              return (
                <div key={entry.id} className="rounded-md border bg-background/60 px-3 py-2">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0 space-y-1">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="truncate text-xs font-medium">{entry.name}</span>
                        <Badge variant="outline" className="px-1.5 py-0 text-[10px]">
                          {entry.feed_type === 'rss' ? 'News' : 'K-line'}
                        </Badge>
                        <Badge variant="outline" className="px-1.5 py-0 text-[10px]">
                          Default source
                        </Badge>
                        {provider && (
                          <Badge
                            variant="outline"
                            className="px-1.5 py-0 text-[10px] text-muted-foreground"
                          >
                            Provider {provider}
                          </Badge>
                        )}
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
                        {fanoutDiagnostic && (
                          <Badge
                            variant="outline"
                            className="border-destructive/40 px-1.5 py-0 text-[10px] text-destructive"
                          >
                            unsafe fan-out
                          </Badge>
                        )}
                      </div>
                      <p className="line-clamp-2 text-[11px] text-muted-foreground">
                        {coverageLabel(entry)}
                      </p>
                      <p className="text-[11px] text-muted-foreground">
                        {entryLifecycleLabel({
                          canEnableInline,
                          needsConfiguration,
                          subscribed,
                        })}
                      </p>
                      {load && <p className="text-[11px] text-muted-foreground">{load}</p>}
                      {fanoutDiagnostic && (
                        <p className="text-[11px] text-destructive">{fanoutDiagnostic}</p>
                      )}
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

function financeBundleActionLabel({
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
  if (canEnableInline) return 'Enable bundle';
  return subscribed ? 'Unwatch' : 'Watch bundle';
}

function entryLifecycleLabel({
  canEnableInline,
  needsConfiguration,
  subscribed,
}: {
  canEnableInline: boolean;
  needsConfiguration: boolean;
  subscribed: boolean;
}): string {
  if (subscribed) return 'This session is watching this source.';
  if (needsConfiguration) return 'Configure this source before ingestion can start.';
  if (canEnableInline)
    return 'Enable source starts ingestion; Watch subscribes this session after it is on.';
  return 'Source is on; Watch subscribes this session without changing ingestion.';
}

function bundleLifecycleLabel({
  bundle,
  canEnableInline,
  needsConfiguration,
  quickStartAvailable,
  subscribed,
}: {
  bundle: FinanceFeedBundle;
  canEnableInline: boolean;
  needsConfiguration: boolean;
  quickStartAvailable: boolean;
  subscribed: boolean;
}): string {
  if (subscribed) return 'This session is watching this bundle.';
  if (needsConfiguration) return 'Configure required sources before this bundle can start.';
  if (canEnableInline) {
    return 'Enable bundle starts ingestion; Watch subscribes this session after sources are on.';
  }
  if (quickStartAvailable) return 'Rara can quick-start this bundle from chat.';
  if (bundle.enabled_source_count < bundle.source_count) {
    return 'Some sources are off; enable or configure them before watching.';
  }
  return 'Sources are on; Watch subscribes this session without changing ingestion.';
}

function quickStartConfigureSourceId(
  hint: FinanceFeedBundleQuickStartHint | undefined,
): string | null {
  const defaultParams = hint?.configure_hints[0]?.default_params;
  const catalogSourceId = defaultParams?.dataFeedCatalogId ?? defaultParams?.catalog_source_id;
  return typeof catalogSourceId === 'string' && catalogSourceId.trim()
    ? catalogSourceId.trim()
    : null;
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

function subscriptionRequestForBundle(
  bundle: FinanceFeedBundle,
  sessionKey: string,
): CreateFinanceSubscriptionRequest {
  const request: CreateFinanceSubscriptionRequest = {
    session_key: sessionKey,
    catalog_source_ids: bundle.catalog_source_ids,
    delivery: 'silent',
  };
  const marketSources = bundle.sources.filter((source) => source.feed_type === 'market_candle');
  if (marketSources.length > 0) {
    request.venues = uniqueStrings(marketSources.map(catalogVenue).filter(isPresent));
    request.symbols = uniqueStrings(marketSources.flatMap(catalogSymbols));
    request.timeframes = uniqueStrings(marketSources.flatMap(catalogTimeframes));
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

function subscriptionMatchesBundle(
  subscription: FinanceSubscription,
  bundle: FinanceFeedBundle,
): boolean {
  const sourceNames = bundle.sources.map(catalogSourceName);
  const eventKinds = uniqueFinanceEventKinds(bundle.sources.map(eventKindForEntry));
  return (
    sourceNames.length > 0 &&
    sourceNames.every((sourceName) => subscription.source_names.includes(sourceName)) &&
    eventKinds.every((kind) => subscription.event_kinds.includes(kind))
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

function catalogProvider(entry: FeedCatalogEntry): string | null {
  return entry.provider?.trim() || null;
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

function formatRate(value: number): string {
  if (Number.isInteger(value)) return value.toString();
  return value.toFixed(2).replace(/\.?0+$/, '');
}

function catalogLoadLabel(entry: FeedCatalogEntry): string | null {
  if (entry.feed_type !== 'market_candle' || !entry.load) return null;

  const parts: string[] = [];
  if (entry.load.configured_market_stream_count != null) {
    parts.push(`${entry.load.configured_market_stream_count} configured streams`);
  }
  if (entry.load.configured_market_poll_request_count != null) {
    parts.push(`${entry.load.configured_market_poll_request_count} req/poll`);
  }
  if (entry.load.configured_market_requests_per_second != null) {
    parts.push(`${formatRate(entry.load.configured_market_requests_per_second)} req/s`);
  }
  if (entry.load.subscribed_market_stream_count > 0) {
    parts.push(`${entry.load.subscribed_market_stream_count} subscribed streams`);
  }

  return parts.length > 0 ? parts.join(' · ') : null;
}

function catalogFanoutDiagnostic(entry: FeedCatalogEntry): string | null {
  if (entry.feed_type !== 'market_candle') return null;
  if (entry.load?.configured_market_fanout_safe_to_start !== false) return null;
  return (
    entry.load.configured_market_fanout_diagnostic ?? 'K-line fan-out exceeds safe polling load.'
  );
}

function bundleLoadLabel(bundle: FinanceFeedBundle): string | null {
  const marketSources = bundle.sources.filter(
    (source) => source.feed_type === 'market_candle' && source.load != null,
  );
  if (marketSources.length === 0) return null;

  const configuredStreams = sumOptional(
    marketSources.map((source) => source.load?.configured_market_stream_count),
  );
  const pollRequests = sumOptional(
    marketSources.map((source) => source.load?.configured_market_poll_request_count),
  );
  const requestsPerSecond = sumOptional(
    marketSources.map((source) => source.load?.configured_market_requests_per_second),
  );
  const subscribedStreams = marketSources.reduce(
    (sum, source) => sum + (source.load?.subscribed_market_stream_count ?? 0),
    0,
  );

  const parts: string[] = [];
  if (configuredStreams != null) parts.push(`${configuredStreams} configured streams`);
  if (pollRequests != null) parts.push(`${pollRequests} req/poll`);
  if (requestsPerSecond != null) parts.push(`${formatRate(requestsPerSecond)} req/s`);
  if (subscribedStreams > 0) parts.push(`${subscribedStreams} subscribed streams`);

  return parts.length > 0 ? parts.join(' · ') : null;
}

function bundleFanoutDiagnostic(bundle: FinanceFeedBundle): string | null {
  const source = bundle.sources.find(
    (candidate) =>
      candidate.feed_type === 'market_candle' &&
      candidate.load?.configured_market_fanout_safe_to_start === false,
  );
  if (!source) return null;
  const diagnostic = catalogFanoutDiagnostic(source);
  return diagnostic ? `${source.name}: ${diagnostic}` : null;
}

function sumOptional(values: Array<number | null | undefined>): number | null {
  let hasValue = false;
  let sum = 0;
  for (const value of values) {
    if (value == null) continue;
    hasValue = true;
    sum += value;
  }
  return hasValue ? sum : null;
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

function isPresent<T>(value: T | null | undefined): value is T {
  return value !== null && value !== undefined;
}

function uniqueStrings(values: string[]): string[] {
  return values.reduce<string[]>((acc, value) => {
    if (!acc.includes(value)) acc.push(value);
    return acc;
  }, []);
}

function uniqueFinanceEventKinds(values: FinanceEventKind[]): FinanceEventKind[] {
  return values.reduce<FinanceEventKind[]>((acc, value) => {
    if (!acc.includes(value)) acc.push(value);
    return acc;
  }, []);
}

function summarizeList(values: string[], max = 3): string {
  if (values.length <= max) return values.join(', ');
  return `${values.slice(0, max).join(', ')} +${values.length - max}`;
}
