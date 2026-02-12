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
import { useParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Skeleton } from "@/components/ui/skeleton";

const BASE_URL = import.meta.env.VITE_API_URL || "";

async function fetchMarkdown(id: string): Promise<string> {
  const resp = await fetch(`${BASE_URL}/api/v1/saved-jobs/${id}/markdown`, {
    headers: {
      Accept: "text/markdown,text/plain",
    },
  });
  if (!resp.ok) {
    const message = await resp.text();
    throw new Error(message || `Failed to fetch markdown (HTTP ${resp.status})`);
  }
  return resp.text();
}

export default function SavedJobMarkdownPreview() {
  const { id } = useParams<{ id: string }>();
  const query = useQuery({
    queryKey: ["saved-job-markdown", id],
    enabled: Boolean(id),
    queryFn: () => fetchMarkdown(id ?? ""),
  });

  const title = useMemo(
    () => (id ? `Markdown Preview · ${id.slice(0, 8)}` : "Markdown Preview"),
    [id],
  );

  if (!id) {
    return <div className="p-8 text-sm text-destructive">Invalid saved job id.</div>;
  }

  return (
    <div className="min-h-screen bg-background text-foreground">
      <div className="mx-auto max-w-4xl px-6 py-8">
        <h1 className="mb-6 text-2xl font-semibold">{title}</h1>

        {query.isLoading && (
          <div className="space-y-3">
            <Skeleton className="h-6 w-1/2" />
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-[90%]" />
            <Skeleton className="h-4 w-[75%]" />
          </div>
        )}

        {query.isError && (
          <p className="text-sm text-destructive">
            Failed to load markdown: {(query.error as Error).message}
          </p>
        )}

        {query.data && (
          <article className="space-y-4 rounded-lg border bg-card p-6 leading-7">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={{
                h1: ({ children }) => <h1 className="mb-4 text-3xl font-bold">{children}</h1>,
                h2: ({ children }) => <h2 className="mb-3 mt-6 text-2xl font-semibold">{children}</h2>,
                h3: ({ children }) => <h3 className="mb-2 mt-5 text-xl font-semibold">{children}</h3>,
                p: ({ children }) => <p className="mb-3 text-base text-foreground/90">{children}</p>,
                ul: ({ children }) => <ul className="mb-3 list-disc space-y-1 pl-6">{children}</ul>,
                ol: ({ children }) => <ol className="mb-3 list-decimal space-y-1 pl-6">{children}</ol>,
                code: ({ className, children }) =>
                  className ? (
                    <code className="block overflow-x-auto rounded-lg bg-muted p-4 text-sm">
                      {children}
                    </code>
                  ) : (
                    <code className="rounded bg-muted px-1.5 py-0.5 text-sm">{children}</code>
                  ),
                blockquote: ({ children }) => (
                  <blockquote className="border-l-4 border-border pl-4 text-muted-foreground">
                    {children}
                  </blockquote>
                ),
                a: ({ href, children }) => (
                  <a
                    href={href}
                    target="_blank"
                    rel="noreferrer"
                    className="text-primary underline underline-offset-2"
                  >
                    {children}
                  </a>
                ),
              }}
            >
              {query.data}
            </ReactMarkdown>
          </article>
        )}
      </div>
    </div>
  );
}
