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

import { BookOpen, Code2, ExternalLink, Sparkles } from "lucide-react";
import { Badge } from "@/components/ui/badge";

interface DocsCardProps {
  title: string;
  description: string;
  href: string;
  icon: React.ReactNode;
  badge: string;
}

function DocsCard({ title, description, href, icon, badge }: DocsCardProps) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="group block rounded-2xl border border-border/70 bg-background/55 p-5 transition-all hover:-translate-y-0.5 hover:border-primary/20 hover:bg-background/80 hover:shadow-md md:p-6"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="inline-flex h-11 w-11 items-center justify-center rounded-2xl border border-primary/15 bg-primary/8 text-primary">
          {icon}
        </div>
        <Badge variant="outline">{badge}</Badge>
      </div>

      <div className="mt-4 space-y-2">
        <h2 className="text-xl font-semibold tracking-tight">{title}</h2>
        <p className="text-sm leading-6 text-muted-foreground">{description}</p>
      </div>

      <div className="mt-5 flex items-center gap-2 text-sm font-medium text-primary transition-transform group-hover:translate-x-0.5">
        Open
        <ExternalLink className="h-4 w-4" />
      </div>
    </a>
  );
}

export default function Docs() {
  return (
    <div className="space-y-6">
      <div className="data-panel p-5 md:p-6">
        <div className="mb-2 inline-flex items-center gap-2 rounded-full border border-primary/15 bg-primary/8 px-3 py-1 text-xs font-medium text-primary">
          <Sparkles className="h-3.5 w-3.5" />
          Docs Hub
        </div>
        <h1 className="text-2xl font-bold tracking-tight">Documentation</h1>
        <p className="mt-2 text-muted-foreground">
          Open project guides or API reference in their dedicated pages.
        </p>

        <div className="mt-6 grid gap-4 lg:grid-cols-2">
          <DocsCard
            title="Guides"
            description="Open the mdBook documentation site for setup guides, architecture notes, and operational docs in a new tab."
            href="/book/"
            icon={<BookOpen className="h-5 w-5" />}
            badge="mdBook"
          />
          <DocsCard
            title="API Reference"
            description="Open Swagger UI in a new tab to browse endpoints, inspect schemas, and test requests against the backend."
            href="/swagger-ui/"
            icon={<Code2 className="h-5 w-5" />}
            badge="Swagger"
          />
        </div>
      </div>
    </div>
  );
}
