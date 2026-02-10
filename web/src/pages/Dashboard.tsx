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

import { Link } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { Briefcase, TrendingUp, Award, XCircle, ArrowRight } from 'lucide-react';
import { api } from '@/api/client';
import type { MetricsSnapshot, DerivedRates, Application } from '@/api/types';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Skeleton } from '@/components/ui/skeleton';
import { Separator } from '@/components/ui/separator';

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

function formatDate(dateStr: string): string {
  const date = new Date(dateStr);
  return date.toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });
}

function statusVariant(
  status: string,
): 'default' | 'secondary' | 'destructive' | 'outline' {
  switch (status.toLowerCase()) {
    case 'offered':
    case 'accepted':
      return 'default';
    case 'rejected':
      return 'destructive';
    case 'interviewing':
    case 'applied':
      return 'secondary';
    default:
      return 'outline';
  }
}

// --- Stat Card Skeleton ---

function StatCardSkeleton() {
  return (
    <Card>
      <CardHeader className="pb-2">
        <Skeleton className="h-4 w-24" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-8 w-16 mb-1" />
        <Skeleton className="h-3 w-32" />
      </CardContent>
    </Card>
  );
}

// --- Stat Card ---

interface StatCardProps {
  title: string;
  value: string;
  description: string;
  icon: React.ReactNode;
}

function StatCard({ title, value, description, icon }: StatCardProps) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">
          {title}
        </CardTitle>
        <span className="text-muted-foreground">{icon}</span>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{value}</div>
        <CardDescription className="mt-1">{description}</CardDescription>
      </CardContent>
    </Card>
  );
}

// --- Error Banner ---

function ErrorBanner({ message }: { message: string }) {
  return (
    <Card className="border-destructive/50 bg-destructive/5">
      <CardContent className="p-4">
        <p className="text-sm text-destructive">{message}</p>
      </CardContent>
    </Card>
  );
}

// --- Empty State ---

function EmptyState({ message }: { message: string }) {
  return (
    <div className="flex flex-col items-center justify-center py-12 text-center">
      <Briefcase className="h-12 w-12 text-muted-foreground/40 mb-4" />
      <p className="text-muted-foreground text-sm">{message}</p>
    </div>
  );
}

// --- Recent Applications Skeleton ---

function RecentApplicationsSkeleton() {
  return (
    <div className="space-y-4">
      {Array.from({ length: 5 }).map((_, i) => (
        <div key={i} className="flex items-center justify-between">
          <div className="space-y-1">
            <Skeleton className="h-4 w-40" />
            <Skeleton className="h-3 w-28" />
          </div>
          <Skeleton className="h-5 w-16" />
        </div>
      ))}
    </div>
  );
}

// --- Dashboard ---

export default function Dashboard() {
  const snapshotQuery = useQuery({
    queryKey: ['analytics', 'snapshot', 'daily'],
    queryFn: () =>
      api.get<MetricsSnapshot>(
        '/api/v1/analytics/snapshots/latest?period=daily',
      ),
    retry: false,
  });

  const ratesQuery = useQuery({
    queryKey: ['analytics', 'rates', snapshotQuery.data?.id],
    queryFn: () =>
      api.get<DerivedRates>(
        `/api/v1/analytics/snapshots/${snapshotQuery.data!.id}/rates`,
      ),
    enabled: !!snapshotQuery.data?.id,
    retry: false,
  });

  const applicationsQuery = useQuery({
    queryKey: ['applications', 'recent'],
    queryFn: () => api.get<Application[]>('/api/v1/applications?limit=5'),
    retry: false,
  });

  const snapshotError = snapshotQuery.isError;
  const ratesError = ratesQuery.isError;
  const hasSnapshot = !!snapshotQuery.data;
  const hasRates = !!ratesQuery.data;

  return (
    <div className="space-y-8">
      {/* Page Header */}
      <div>
        <h1 className="text-2xl font-bold">Dashboard</h1>
        <p className="text-muted-foreground mt-2">
          Overview of your job search pipeline.
        </p>
      </div>

      {/* Stat Cards */}
      {snapshotError && !hasSnapshot ? (
        <ErrorBanner message="Unable to load analytics data. Make sure the API server is running." />
      ) : null}

      <div className="grid gap-4 grid-cols-1 sm:grid-cols-2 lg:grid-cols-4">
        {snapshotQuery.isLoading ? (
          <>
            <StatCardSkeleton />
            <StatCardSkeleton />
            <StatCardSkeleton />
            <StatCardSkeleton />
          </>
        ) : hasSnapshot ? (
          <>
            <StatCard
              title="Total Applications"
              value={String(snapshotQuery.data.total_applications)}
              description={`As of ${formatDate(snapshotQuery.data.snapshot_date)}`}
              icon={<Briefcase className="h-4 w-4" />}
            />
            <StatCard
              title="Interview Rate"
              value={
                hasRates
                  ? formatPercent(ratesQuery.data.interview_rate)
                  : ratesError
                    ? '--'
                    : '...'
              }
              description="Interviews / Applications"
              icon={<TrendingUp className="h-4 w-4" />}
            />
            <StatCard
              title="Offer Rate"
              value={
                hasRates
                  ? formatPercent(ratesQuery.data.offer_rate)
                  : ratesError
                    ? '--'
                    : '...'
              }
              description="Offers / Applications"
              icon={<Award className="h-4 w-4" />}
            />
            <StatCard
              title="Total Rejections"
              value={String(snapshotQuery.data.total_rejections)}
              description={`${snapshotQuery.data.total_interviews} interviews conducted`}
              icon={<XCircle className="h-4 w-4" />}
            />
          </>
        ) : (
          <>
            <StatCard
              title="Total Applications"
              value="0"
              description="No data yet"
              icon={<Briefcase className="h-4 w-4" />}
            />
            <StatCard
              title="Interview Rate"
              value="--"
              description="No data yet"
              icon={<TrendingUp className="h-4 w-4" />}
            />
            <StatCard
              title="Offer Rate"
              value="--"
              description="No data yet"
              icon={<Award className="h-4 w-4" />}
            />
            <StatCard
              title="Total Rejections"
              value="0"
              description="No data yet"
              icon={<XCircle className="h-4 w-4" />}
            />
          </>
        )}
      </div>

      <Separator />

      {/* Recent Applications */}
      <div>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">Recent Applications</h2>
          <Link
            to="/applications"
            className="text-sm text-muted-foreground hover:text-foreground inline-flex items-center gap-1 transition-colors"
          >
            View all
            <ArrowRight className="h-3 w-3" />
          </Link>
        </div>

        {applicationsQuery.isLoading ? (
          <RecentApplicationsSkeleton />
        ) : applicationsQuery.isError ? (
          <ErrorBanner message="Unable to load recent applications." />
        ) : applicationsQuery.data && applicationsQuery.data.length > 0 ? (
          <Card>
            <CardContent className="p-0">
              <div className="divide-y">
                {applicationsQuery.data.map((app) => (
                  <div
                    key={app.id}
                    className="flex items-center justify-between px-6 py-4"
                  >
                    <div className="min-w-0 flex-1">
                      <p className="text-sm font-medium truncate">
                        {app.company_name}
                      </p>
                      <p className="text-sm text-muted-foreground truncate">
                        {app.position_title}
                      </p>
                    </div>
                    <div className="flex items-center gap-4 ml-4 shrink-0">
                      <span className="text-xs text-muted-foreground hidden sm:inline">
                        {formatDate(app.created_at)}
                      </span>
                      <Badge variant={statusVariant(app.status)}>
                        {app.status}
                      </Badge>
                    </div>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        ) : (
          <EmptyState message="No applications yet. Start by adding your first job application." />
        )}
      </div>
    </div>
  );
}
