import { useState } from "react";
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
import { Search, Briefcase, Info } from "lucide-react";

const JOB_SITES = [
  { id: "linkedin", label: "LinkedIn" },
  { id: "indeed", label: "Indeed" },
  { id: "glassdoor", label: "Glassdoor" },
  { id: "google", label: "Google" },
  { id: "ziprecruiter", label: "ZipRecruiter" },
] as const;

export default function JobDiscovery() {
  const [keywords, setKeywords] = useState("");
  const [location, setLocation] = useState("");
  const [jobType, setJobType] = useState<string>("");
  const [maxResults, setMaxResults] = useState("20");
  const [selectedSites, setSelectedSites] = useState<string[]>([
    "linkedin",
    "indeed",
  ]);
  const [submitted, setSubmitted] = useState(false);

  const toggleSite = (siteId: string) => {
    setSelectedSites((prev) =>
      prev.includes(siteId)
        ? prev.filter((s) => s !== siteId)
        : [...prev, siteId]
    );
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitted(true);
  };

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
                />
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

            <Button type="submit" disabled={!keywords.trim()}>
              <Search className="h-4 w-4 mr-2" />
              Search Jobs
            </Button>
          </form>
        </CardContent>
      </Card>

      {submitted && (
        <Card className="border-blue-200 bg-blue-50/50">
          <CardContent className="flex items-start gap-3 p-6">
            <Info className="h-5 w-5 text-blue-600 mt-0.5 shrink-0" />
            <div>
              <p className="font-medium text-blue-900">
                Job discovery API coming soon
              </p>
              <p className="text-sm text-blue-700 mt-1">
                This will search across selected job boards (
                {selectedSites
                  .map(
                    (s) => JOB_SITES.find((site) => site.id === s)?.label ?? s
                  )
                  .join(", ")}
                ) for "{keywords}" positions
                {location ? ` in ${location}` : ""}.
              </p>
            </div>
          </CardContent>
        </Card>
      )}

      <Card>
        <CardContent className="flex flex-col items-center justify-center py-12 text-muted-foreground">
          <Briefcase className="h-12 w-12 mb-4 opacity-50" />
          <p className="text-lg font-medium">Search results will appear here</p>
          <p className="text-sm">
            Fill in the search form above to discover job opportunities.
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
