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

import { useMemo } from "react";
import { useMutation } from "@tanstack/react-query";
import { useLocalStorage } from "@/hooks/use-local-storage";
import {
  POPULAR_LOCATIONS,
  RECENT_LOCATIONS_KEY,
  MAX_RECENT_LOCATIONS,
} from "@/data/locations";
import { api } from "@/api/client";
import type { DiscoveryCriteria, NormalizedJob } from "@/api/types";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Search,
  Briefcase,
  ExternalLink,
  AlertCircle,
  Loader2,
  MapPin,
  Building2,
  DollarSign,
  Calendar,
} from "lucide-react";

const JOB_SITES = [
  { id: "linkedin", label: "LinkedIn" },
  { id: "indeed", label: "Indeed" },
  { id: "glassdoor", label: "Glassdoor" },
  { id: "google", label: "Google" },
  { id: "ziprecruiter", label: "ZipRecruiter" },
] as const;

function formatSalary(
  min?: number,
  max?: number,
  currency?: string
): string | null {
  if (!min && !max) return null;
  const cur = currency || "USD";
  if (min && max) return `${cur} ${min.toLocaleString()} - ${max.toLocaleString()}`;
  if (min) return `${cur} ${min.toLocaleString()}+`;
  return `Up to ${cur} ${max!.toLocaleString()}`;
}

function formatDate(dateStr?: string): string | null {
  if (!dateStr) return null;
  try {
    return new Date(dateStr).toLocaleDateString();
  } catch {
    return dateStr;
  }
}

export default function JobDiscovery() {
  const [keywords, setKeywords] = useLocalStorage("job-discovery-keywords", "");
  const [location, setLocation] = useLocalStorage("job-discovery-location", "");
  const [jobType, setJobType] = useLocalStorage("job-discovery-job-type", "");
  const [maxResults, setMaxResults] = useLocalStorage("job-discovery-max-results", "20");
  const [selectedSites, setSelectedSites] = useLocalStorage<string[]>(
    "job-discovery-selected-sites",
    ["linkedin", "indeed"],
  );

  const locationSuggestions = useMemo(() => {
    const recent: string[] = [];
    try {
      const stored = window.localStorage.getItem(RECENT_LOCATIONS_KEY);
      if (stored) recent.push(...(JSON.parse(stored) as string[]));
    } catch { /* ignore */ }
    const seen = new Set(recent.map((l) => l.toLowerCase()));
    const staticFiltered = POPULAR_LOCATIONS.filter(
      (l) => !seen.has(l.toLowerCase()),
    );
    return [...recent, ...staticFiltered];
  }, []);

  const discoverMutation = useMutation<NormalizedJob[], Error, DiscoveryCriteria>({
    mutationFn: (criteria) =>
      api.post<NormalizedJob[]>("/api/v1/jobs/discover", criteria),
  });

  const toggleSite = (siteId: string) => {
    setSelectedSites((prev) =>
      prev.includes(siteId)
        ? prev.filter((s) => s !== siteId)
        : [...prev, siteId]
    );
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const keywordList = keywords
      .split(/[,\s]+/)
      .map((k) => k.trim())
      .filter(Boolean);

    const criteria: DiscoveryCriteria = {
      keywords: keywordList,
      location: location || undefined,
      job_type: jobType || undefined,
      max_results: parseInt(maxResults, 10) || undefined,
      sites: selectedSites.length > 0 ? selectedSites : undefined,
    };

    // Save location to recent locations
    if (location.trim()) {
      try {
        const stored = window.localStorage.getItem(RECENT_LOCATIONS_KEY);
        const recent: string[] = stored ? (JSON.parse(stored) as string[]) : [];
        const trimmed = location.trim();
        const updated = [
          trimmed,
          ...recent.filter((l) => l.toLowerCase() !== trimmed.toLowerCase()),
        ].slice(0, MAX_RECENT_LOCATIONS);
        window.localStorage.setItem(RECENT_LOCATIONS_KEY, JSON.stringify(updated));
      } catch { /* ignore */ }
    }

    discoverMutation.mutate(criteria);
  };

  const jobs = discoverMutation.data;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Job Discovery</h1>
        <p className="text-muted-foreground mt-2">
          Discover new job opportunities matched by AI.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Search className="h-5 w-5" />
            Search Jobs
          </CardTitle>
          <CardDescription>
            Configure your job search criteria across multiple job boards.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="keywords">Keywords *</Label>
                <Input
                  id="keywords"
                  placeholder="e.g. Rust developer, Backend engineer"
                  value={keywords}
                  onChange={(e) => setKeywords(e.target.value)}
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="location">Location</Label>
                <Input
                  id="location"
                  placeholder="e.g. San Francisco, Remote"
                  value={location}
                  onChange={(e) => setLocation(e.target.value)}
                  list="location-suggestions"
                  autoComplete="off"
                />
                <datalist id="location-suggestions">
                  {locationSuggestions.map((loc) => (
                    <option key={loc} value={loc} />
                  ))}
                </datalist>
              </div>

              <div className="space-y-2">
                <Label htmlFor="job-type">Job Type</Label>
                <Select value={jobType} onValueChange={setJobType}>
                  <SelectTrigger id="job-type">
                    <SelectValue placeholder="Select type" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="full-time">Full-time</SelectItem>
                    <SelectItem value="part-time">Part-time</SelectItem>
                    <SelectItem value="internship">Internship</SelectItem>
                    <SelectItem value="contract">Contract</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="max-results">Max Results</Label>
                <Input
                  id="max-results"
                  type="number"
                  min={1}
                  max={100}
                  value={maxResults}
                  onChange={(e) => setMaxResults(e.target.value)}
                />
              </div>
            </div>

            <div className="space-y-2">
              <Label>Job Sites</Label>
              <div className="flex flex-wrap gap-2">
                {JOB_SITES.map((site) => {
                  const isSelected = selectedSites.includes(site.id);
                  return (
                    <Button
                      key={site.id}
                      type="button"
                      variant={isSelected ? "default" : "outline"}
                      size="sm"
                      onClick={() => toggleSite(site.id)}
                    >
                      {site.label}
                    </Button>
                  );
                })}
              </div>
            </div>

            <Button
              type="submit"
              disabled={!keywords.trim() || discoverMutation.isPending}
            >
              {discoverMutation.isPending ? (
                <Loader2 className="h-4 w-4 mr-2 animate-spin" />
              ) : (
                <Search className="h-4 w-4 mr-2" />
              )}
              {discoverMutation.isPending ? "Searching..." : "Search Jobs"}
            </Button>
          </form>
        </CardContent>
      </Card>

      {/* Error state */}
      {discoverMutation.isError && (
        <Card className="border-red-200 bg-red-50/50">
          <CardContent className="flex items-start gap-3 p-6">
            <AlertCircle className="h-5 w-5 text-red-600 mt-0.5 shrink-0" />
            <div>
              <p className="font-medium text-red-900">Search failed</p>
              <p className="text-sm text-red-700 mt-1">
                {discoverMutation.error.message}
              </p>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Loading state */}
      {discoverMutation.isPending && (
        <Card>
          <CardContent className="p-6 space-y-4">
            {Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="space-y-3">
                <Skeleton className="h-5 w-2/3" />
                <Skeleton className="h-4 w-1/3" />
                <Skeleton className="h-4 w-1/2" />
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      {/* Results */}
      {jobs && jobs.length > 0 && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-lg font-semibold">
              Found {jobs.length} job{jobs.length !== 1 ? "s" : ""}
            </h2>
          </div>
          {jobs.map((job) => {
            const salary = formatSalary(
              job.salary_min,
              job.salary_max,
              job.salary_currency
            );
            const posted = formatDate(job.posted_at);
            return (
              <Card key={job.id}>
                <CardContent className="p-6">
                  <div className="flex items-start justify-between gap-4">
                    <div className="space-y-2 flex-1 min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <h3 className="text-lg font-semibold">{job.title}</h3>
                        <Badge variant="outline">{job.source_name}</Badge>
                      </div>

                      <div className="flex items-center gap-4 text-sm text-muted-foreground flex-wrap">
                        <span className="flex items-center gap-1">
                          <Building2 className="h-4 w-4" />
                          {job.company}
                        </span>
                        {job.location && (
                          <span className="flex items-center gap-1">
                            <MapPin className="h-4 w-4" />
                            {job.location}
                          </span>
                        )}
                        {salary && (
                          <span className="flex items-center gap-1">
                            <DollarSign className="h-4 w-4" />
                            {salary}
                          </span>
                        )}
                        {posted && (
                          <span className="flex items-center gap-1">
                            <Calendar className="h-4 w-4" />
                            {posted}
                          </span>
                        )}
                      </div>

                      {job.tags && job.tags.length > 0 && (
                        <div className="flex flex-wrap gap-1 mt-2">
                          {job.tags.map((tag) => (
                            <Badge key={tag} variant="secondary" className="text-xs">
                              {tag}
                            </Badge>
                          ))}
                        </div>
                      )}
                    </div>

                    {job.url && (
                      <a
                        href={job.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="shrink-0"
                      >
                        <Button variant="outline" size="sm">
                          <ExternalLink className="h-4 w-4 mr-1" />
                          View
                        </Button>
                      </a>
                    )}
                  </div>
                </CardContent>
              </Card>
            );
          })}
        </div>
      )}

      {/* Empty results after search */}
      {jobs && jobs.length === 0 && (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12 text-muted-foreground">
            <Briefcase className="h-12 w-12 mb-4 opacity-50" />
            <p className="text-lg font-medium">No jobs found</p>
            <p className="text-sm">
              Try adjusting your search criteria or keywords.
            </p>
          </CardContent>
        </Card>
      )}

      {/* Initial state - no search performed yet */}
      {!jobs && !discoverMutation.isPending && !discoverMutation.isError && (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12 text-muted-foreground">
            <Briefcase className="h-12 w-12 mb-4 opacity-50" />
            <p className="text-lg font-medium">
              Search results will appear here
            </p>
            <p className="text-sm">
              Fill in the search form above to discover job opportunities.
            </p>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
