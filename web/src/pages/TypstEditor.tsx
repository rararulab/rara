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

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams, useNavigate } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter } from "@codemirror/view";
import { markdown } from "@codemirror/lang-markdown";
import { oneDark } from "@codemirror/theme-one-dark";
import { api } from "@/api/client";
import type { TypstProject, FileEntry, FileContent, RenderResult, JustRecipe, RunOutput } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  Loader2,
  Play,
  History,
  Download,
  Save,
  Terminal,
  CheckCircle,
  XCircle,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Recursive File Tree (left column)
// ---------------------------------------------------------------------------

function FileTreeNode({
  entry,
  activeFilePath,
  onSelect,
  depth,
}: {
  entry: FileEntry;
  activeFilePath: string | null;
  onSelect: (path: string) => void;
  depth: number;
}) {
  const [expanded, setExpanded] = useState(depth < 2);

  if (entry.is_dir) {
    return (
      <div>
        <button
          type="button"
          className="flex items-center gap-1 w-full rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground transition-colors"
          style={{ paddingLeft: `${depth * 12 + 8}px` }}
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? (
            <ChevronDown className="h-3 w-3 shrink-0" />
          ) : (
            <ChevronRight className="h-3 w-3 shrink-0" />
          )}
          <Folder className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate font-mono text-xs">
            {entry.path.split("/").pop()}
          </span>
        </button>
        {expanded && entry.children && (
          <div>
            {entry.children.map((child) => (
              <FileTreeNode
                key={child.path}
                entry={child}
                activeFilePath={activeFilePath}
                onSelect={onSelect}
                depth={depth + 1}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <button
      type="button"
      className={cn(
        "flex items-center gap-1.5 w-full rounded-md px-2 py-1.5 text-sm cursor-pointer transition-colors",
        activeFilePath === entry.path
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground"
      )}
      style={{ paddingLeft: `${depth * 12 + 8}px` }}
      onClick={() => onSelect(entry.path)}
    >
      <File className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate font-mono text-xs">
        {entry.path.split("/").pop()}
      </span>
    </button>
  );
}

function FileTree({
  entries,
  activeFilePath,
  onSelect,
  isLoading,
}: {
  entries: FileEntry[];
  activeFilePath: string | null;
  onSelect: (path: string) => void;
  isLoading: boolean;
}) {
  return (
    <div className="flex flex-col border-r bg-card w-56 shrink-0">
      <div className="flex items-center justify-between border-b px-3 py-2">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Files
        </h3>
      </div>
      <div className="flex-1 overflow-y-auto p-1">
        {isLoading ? (
          <div className="space-y-1 p-1">
            {Array.from({ length: 3 }).map((_, i) => (
              <Skeleton key={i} className="h-7 w-full" />
            ))}
          </div>
        ) : entries.length === 0 ? (
          <div className="p-3 text-center text-xs text-muted-foreground">
            No files found.
          </div>
        ) : (
          <div className="space-y-0.5">
            {entries.map((entry) => (
              <FileTreeNode
                key={entry.path}
                entry={entry}
                activeFilePath={activeFilePath}
                onSelect={onSelect}
                depth={0}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// CodeMirror Editor (center column)
// ---------------------------------------------------------------------------

function CodeEditor({
  content,
  onContentChange,
  onSave,
  isSaving,
}: {
  content: string;
  onContentChange: (value: string) => void;
  onSave: () => void;
  isSaving: boolean;
}) {
  const editorRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onContentChangeRef = useRef(onContentChange);
  const onSaveRef = useRef(onSave);

  useEffect(() => {
    onContentChangeRef.current = onContentChange;
  }, [onContentChange]);

  useEffect(() => {
    onSaveRef.current = onSave;
  }, [onSave]);

  useEffect(() => {
    if (!editorRef.current) return;

    const state = EditorState.create({
      doc: content,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        markdown(),
        oneDark,
        keymap.of([
          {
            key: "Mod-s",
            run: () => {
              onSaveRef.current();
              return true;
            },
          },
        ]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onContentChangeRef.current(update.state.doc.toString());
          }
        }),
        EditorView.theme({
          "&": { height: "100%" },
          ".cm-scroller": { overflow: "auto" },
        }),
      ],
    });

    const view = new EditorView({
      state,
      parent: editorRef.current,
    });

    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [content]);

  return (
    <div className="flex flex-col flex-1 min-w-0">
      <div className="flex items-center justify-between border-b px-3 py-1.5">
        <span className="text-xs text-muted-foreground">Editor</span>
        <div className="flex items-center gap-1">
          {isSaving && (
            <span className="flex items-center gap-1 text-xs text-muted-foreground">
              <Loader2 className="h-3 w-3 animate-spin" />
              Saving...
            </span>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs"
            onClick={onSave}
            title="Save (Ctrl+S)"
          >
            <Save className="h-3.5 w-3.5 mr-1" />
            Save
          </Button>
        </div>
      </div>
      <div ref={editorRef} className="flex-1 min-h-0 overflow-hidden" />
    </div>
  );
}

// ---------------------------------------------------------------------------
// PDF Preview (right column)
// ---------------------------------------------------------------------------

function PdfPreview({
  renders,
  isCompiling,
  onCompile,
  isRendersLoading,
}: {
  projectId: string;
  renders: RenderResult[];
  isCompiling: boolean;
  onCompile: () => void;
  isRendersLoading: boolean;
}) {
  const [selectedRenderId, setSelectedRenderId] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [pdfUrl, setPdfUrl] = useState<string | null>(null);
  const [isLoadingPdf, setIsLoadingPdf] = useState(false);

  const displayRenderId = selectedRenderId ?? (renders.length > 0 ? renders[0].id : null);

  useEffect(() => {
    if (!displayRenderId) {
      setPdfUrl(null);
      return;
    }

    let cancelled = false;
    setIsLoadingPdf(true);

    api
      .blob(`/api/v1/typst/renders/${displayRenderId}/pdf`)
      .then((blob) => {
        if (cancelled) return;
        const url = URL.createObjectURL(blob);
        setPdfUrl((prev) => {
          if (prev) URL.revokeObjectURL(prev);
          return url;
        });
      })
      .catch(() => {
        if (!cancelled) setPdfUrl(null);
      })
      .finally(() => {
        if (!cancelled) setIsLoadingPdf(false);
      });

    return () => {
      cancelled = true;
    };
  }, [displayRenderId]);

  useEffect(() => {
    return () => {
      if (pdfUrl) URL.revokeObjectURL(pdfUrl);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (renders.length > 0 && selectedRenderId) {
      const found = renders.find((r) => r.id === selectedRenderId);
      if (!found) setSelectedRenderId(null);
    }
  }, [renders, selectedRenderId]);

  function formatDate(dateStr: string) {
    return new Date(dateStr).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function formatFileSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="flex items-center justify-between border-b px-3 py-1.5">
        <span className="text-xs text-muted-foreground">Preview</span>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs"
            onClick={() => setShowHistory((v) => !v)}
            title="Render history"
          >
            <History className="h-3.5 w-3.5 mr-1" />
            History
            {renders.length > 0 && (
              <span className="ml-1 text-muted-foreground">
                ({renders.length})
              </span>
            )}
          </Button>
          <Button
            size="sm"
            className="h-7 text-xs"
            onClick={onCompile}
            disabled={isCompiling}
          >
            {isCompiling ? (
              <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />
            ) : (
              <Play className="h-3.5 w-3.5 mr-1" />
            )}
            {isCompiling ? "Compiling..." : "Compile"}
          </Button>
        </div>
      </div>

      {showHistory && (
        <div className="border-b max-h-48 overflow-y-auto">
          {isRendersLoading ? (
            <div className="space-y-1 p-2">
              {Array.from({ length: 2 }).map((_, i) => (
                <Skeleton key={i} className="h-8 w-full" />
              ))}
            </div>
          ) : renders.length === 0 ? (
            <div className="p-3 text-center text-xs text-muted-foreground">
              No renders yet. Click &quot;Compile&quot; to generate a PDF.
            </div>
          ) : (
            <div className="space-y-0.5 p-1">
              {renders.map((render, idx) => (
                <button
                  type="button"
                  key={render.id}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors",
                    displayRenderId === render.id
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-accent/50"
                  )}
                  onClick={() => setSelectedRenderId(render.id)}
                >
                  <ChevronRight className="h-3 w-3 shrink-0" />
                  <div className="flex-1 min-w-0">
                    <p className="truncate font-medium">
                      {idx === 0 ? "Latest" : `Render #${renders.length - idx}`}
                    </p>
                    <p className="text-muted-foreground">
                      {formatDate(render.created_at)} - {render.page_count} page
                      {render.page_count !== 1 ? "s" : ""} -{" "}
                      {formatFileSize(render.file_size)}
                    </p>
                  </div>
                  <a
                    href={`/api/v1/typst/renders/${render.id}/pdf`}
                    download
                    className="shrink-0 rounded p-1 hover:bg-background"
                    onClick={(e) => e.stopPropagation()}
                    title="Download PDF"
                  >
                    <Download className="h-3 w-3" />
                  </a>
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      <div className="flex-1 min-h-0 bg-muted/30">
        {isLoadingPdf || isCompiling ? (
          <div className="flex h-full items-center justify-center">
            <div className="flex flex-col items-center gap-2 text-muted-foreground">
              <Loader2 className="h-8 w-8 animate-spin" />
              <span className="text-sm">
                {isCompiling ? "Compiling..." : "Loading PDF..."}
              </span>
            </div>
          </div>
        ) : pdfUrl ? (
          <iframe
            src={pdfUrl}
            className="h-full w-full"
            title="PDF Preview"
          />
        ) : (
          <div className="flex h-full items-center justify-center">
            <div className="flex flex-col items-center gap-2 text-muted-foreground">
              <File className="h-12 w-12 opacity-20" />
              <p className="text-sm">
                No PDF yet. Click &quot;Compile&quot; to render.
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tasks Panel (bottom of the right column)
// ---------------------------------------------------------------------------

function TasksPanel({ projectId }: { projectId: string }) {
  const [expanded, setExpanded] = useState(true);
  const [customCommand, setCustomCommand] = useState("");
  const [lastOutput, setLastOutput] = useState<RunOutput | null>(null);
  const [isRunning, setIsRunning] = useState(false);

  const recipesQuery = useQuery({
    queryKey: ["typst-recipes", projectId],
    queryFn: () => api.listRecipes(projectId),
    enabled: !!projectId,
    // Don't error on 500 (no justfile) — treat as empty
    retry: false,
  });

  const recipes: JustRecipe[] = recipesQuery.data ?? [];
  const hasRecipes = recipes.length > 0;

  async function handleRunRecipe(recipeName: string) {
    setIsRunning(true);
    try {
      const output = await api.runProjectCommand(projectId, {
        recipe: recipeName,
      });
      setLastOutput(output);
    } catch (err) {
      setLastOutput({
        exit_code: -1,
        stdout: "",
        stderr: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setIsRunning(false);
    }
  }

  async function handleRunCommand() {
    if (!customCommand.trim()) return;
    setIsRunning(true);
    try {
      const output = await api.runProjectCommand(projectId, {
        command: customCommand,
      });
      setLastOutput(output);
    } catch (err) {
      setLastOutput({
        exit_code: -1,
        stdout: "",
        stderr: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setIsRunning(false);
    }
  }

  return (
    <div className="border-t flex flex-col">
      {/* Header */}
      <button
        type="button"
        className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground hover:bg-accent/50 transition-colors"
        onClick={() => setExpanded((v) => !v)}
      >
        {expanded ? (
          <ChevronDown className="h-3 w-3" />
        ) : (
          <ChevronRight className="h-3 w-3" />
        )}
        <Terminal className="h-3.5 w-3.5" />
        Tasks
      </button>

      {expanded && (
        <div className="flex flex-col gap-2 px-3 pb-3 max-h-72 overflow-y-auto">
          {/* Just recipes list */}
          {hasRecipes && (
            <div className="space-y-0.5">
              {recipes.map((recipe) => (
                <button
                  key={recipe.name}
                  type="button"
                  className="flex items-center gap-2 w-full rounded-md px-2 py-1.5 text-left text-xs hover:bg-accent/50 transition-colors disabled:opacity-50"
                  onClick={() => handleRunRecipe(recipe.name)}
                  disabled={isRunning}
                >
                  <Play className="h-3 w-3 shrink-0 text-green-500" />
                  <span className="font-mono font-medium">{recipe.name}</span>
                  {recipe.description && (
                    <span className="text-muted-foreground truncate">
                      # {recipe.description}
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}

          {/* No justfile message */}
          {!recipesQuery.isLoading && !hasRecipes && (
            <p className="text-xs text-muted-foreground">
              No justfile found. Use the command input below.
            </p>
          )}

          {/* Custom command input */}
          <div className="flex items-center gap-1.5">
            <input
              type="text"
              value={customCommand}
              onChange={(e) => setCustomCommand(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !isRunning) handleRunCommand();
              }}
              placeholder="Custom command..."
              className="flex-1 rounded-md border bg-background px-2 py-1 text-xs font-mono placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
              disabled={isRunning}
            />
            <Button
              variant="outline"
              size="sm"
              className="h-7 text-xs"
              onClick={handleRunCommand}
              disabled={isRunning || !customCommand.trim()}
            >
              {isRunning ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                "Run"
              )}
            </Button>
          </div>

          {/* Output area */}
          {lastOutput && (
            <div className="rounded-md border bg-zinc-950 p-2 text-xs font-mono">
              {lastOutput.stdout && (
                <pre className="whitespace-pre-wrap text-zinc-300 max-h-32 overflow-y-auto">
                  {lastOutput.stdout}
                </pre>
              )}
              {lastOutput.stderr && (
                <pre className="whitespace-pre-wrap text-red-400 max-h-32 overflow-y-auto">
                  {lastOutput.stderr}
                </pre>
              )}
              <div className="mt-1.5 flex items-center gap-1 border-t border-zinc-800 pt-1.5">
                {lastOutput.exit_code === 0 ? (
                  <CheckCircle className="h-3 w-3 text-green-500" />
                ) : (
                  <XCircle className="h-3 w-3 text-red-500" />
                )}
                <span
                  className={cn(
                    "text-xs",
                    lastOutput.exit_code === 0
                      ? "text-green-500"
                      : "text-red-500"
                  )}
                >
                  Exit code: {lastOutput.exit_code}
                </span>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helper: collect all file paths from a nested FileEntry tree
// ---------------------------------------------------------------------------

function collectFilePaths(entries: FileEntry[]): string[] {
  const paths: string[] = [];
  for (const entry of entries) {
    if (entry.is_dir && entry.children) {
      paths.push(...collectFilePaths(entry.children));
    } else if (!entry.is_dir) {
      paths.push(entry.path);
    }
  }
  return paths;
}

// ---------------------------------------------------------------------------
// TypstEditor (main page)
// ---------------------------------------------------------------------------

export default function TypstEditor() {
  const { projectId } = useParams<{ projectId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [activeFilePath, setActiveFilePath] = useState<string | null>(null);
  const [editorContent, setEditorContent] = useState<string>("");
  const [editorKey, setEditorKey] = useState(0);

  // Debounce auto-save timer
  const autoSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingContentRef = useRef<string | null>(null);

  // Fetch project details
  const projectQuery = useQuery({
    queryKey: ["typst-project", projectId],
    queryFn: () => api.get<TypstProject>(`/api/v1/typst/projects/${projectId}`),
    enabled: !!projectId,
  });

  // Fetch project file tree
  const filesQuery = useQuery({
    queryKey: ["typst-files", projectId],
    queryFn: () =>
      api.get<FileEntry[]>(`/api/v1/typst/projects/${projectId}/files`),
    enabled: !!projectId,
  });

  // Fetch renders
  const rendersQuery = useQuery({
    queryKey: ["typst-renders", projectId],
    queryFn: () =>
      api.get<RenderResult[]>(
        `/api/v1/typst/projects/${projectId}/renders`
      ),
    enabled: !!projectId,
  });

  const fileEntries = filesQuery.data ?? [];
  const allFilePaths = useMemo(
    () => collectFilePaths(fileEntries),
    [fileEntries]
  );
  const renders = useMemo(
    () =>
      [...(rendersQuery.data ?? [])].sort(
        (a, b) =>
          new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
      ),
    [rendersQuery.data]
  );

  // Auto-select main file on first load
  useEffect(() => {
    if (allFilePaths.length > 0 && !activeFilePath) {
      const mainFile = projectQuery.data?.main_file ?? "main.typ";
      const found = allFilePaths.find((p) => p === mainFile);
      if (found) {
        loadFile(found);
      } else {
        loadFile(allFilePaths[0]);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allFilePaths, activeFilePath, projectQuery.data?.main_file]);

  // Load file content from the API
  function loadFile(path: string) {
    setActiveFilePath(path);
    api
      .get<FileContent>(
        `/api/v1/typst/projects/${projectId}/files/${encodeURIComponent(path)}`
      )
      .then((fc) => {
        setEditorContent(fc.content);
        setEditorKey((k) => k + 1);
      });
  }

  // Save file mutation
  const saveMutation = useMutation({
    mutationFn: ({ path, content }: { path: string; content: string }) =>
      api.put<FileContent>(
        `/api/v1/typst/projects/${projectId}/files/${encodeURIComponent(path)}`,
        { content }
      ),
  });

  // Compile mutation
  const compileMutation = useMutation({
    mutationFn: () =>
      api.post<RenderResult>(
        `/api/v1/typst/projects/${projectId}/compile`,
        { main_file: null }
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["typst-renders", projectId],
      });
    },
  });

  // Handle file selection
  const handleSelectFile = useCallback(
    (path: string) => {
      // Save current file before switching, if changed
      if (activeFilePath && pendingContentRef.current !== null) {
        saveMutation.mutate({
          path: activeFilePath,
          content: pendingContentRef.current,
        });
        pendingContentRef.current = null;
        if (autoSaveTimerRef.current) {
          clearTimeout(autoSaveTimerRef.current);
          autoSaveTimerRef.current = null;
        }
      }

      loadFile(path);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [activeFilePath, saveMutation]
  );

  // Handle content change with debounced auto-save
  const handleContentChange = useCallback(
    (value: string) => {
      pendingContentRef.current = value;

      if (autoSaveTimerRef.current) {
        clearTimeout(autoSaveTimerRef.current);
      }

      autoSaveTimerRef.current = setTimeout(() => {
        if (activeFilePath && pendingContentRef.current !== null) {
          saveMutation.mutate({
            path: activeFilePath,
            content: pendingContentRef.current,
          });
          pendingContentRef.current = null;
        }
      }, 1000);
    },
    [activeFilePath, saveMutation]
  );

  // Handle manual save
  const handleSave = useCallback(() => {
    if (autoSaveTimerRef.current) {
      clearTimeout(autoSaveTimerRef.current);
      autoSaveTimerRef.current = null;
    }

    if (activeFilePath && pendingContentRef.current !== null) {
      saveMutation.mutate({
        path: activeFilePath,
        content: pendingContentRef.current,
      });
      pendingContentRef.current = null;
    }
  }, [activeFilePath, saveMutation]);

  // Handle compile
  const handleCompile = useCallback(() => {
    // Save before compiling
    if (activeFilePath && pendingContentRef.current !== null) {
      if (autoSaveTimerRef.current) {
        clearTimeout(autoSaveTimerRef.current);
        autoSaveTimerRef.current = null;
      }
      saveMutation.mutate(
        {
          path: activeFilePath,
          content: pendingContentRef.current,
        },
        {
          onSuccess: () => {
            compileMutation.mutate();
          },
        }
      );
      pendingContentRef.current = null;
    } else {
      compileMutation.mutate();
    }
  }, [activeFilePath, saveMutation, compileMutation]);

  // Cleanup timer on unmount
  useEffect(() => {
    return () => {
      if (autoSaveTimerRef.current) {
        clearTimeout(autoSaveTimerRef.current);
      }
    };
  }, []);

  if (!projectId) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        No project selected.
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {/* Top bar */}
      <div className="flex items-center gap-3 border-b px-4 py-2 bg-card">
        <Button
          variant="ghost"
          size="sm"
          className="h-7"
          onClick={() => navigate("/typst")}
        >
          <ArrowLeft className="h-3.5 w-3.5 mr-1" />
          Projects
        </Button>
        {projectQuery.isLoading ? (
          <Skeleton className="h-5 w-32" />
        ) : (
          <h2 className="text-sm font-semibold truncate">
            {projectQuery.data?.name ?? "Untitled"}
          </h2>
        )}
        {projectQuery.data?.local_path && (
          <span className="text-xs text-muted-foreground truncate hidden md:inline font-mono">
            {projectQuery.data.local_path}
          </span>
        )}
      </div>

      {/* Three-column layout */}
      <div className="flex flex-1 min-h-0">
        {/* Left: File tree */}
        <FileTree
          entries={fileEntries}
          activeFilePath={activeFilePath}
          onSelect={handleSelectFile}
          isLoading={filesQuery.isLoading}
        />

        {/* Center: Code editor */}
        {activeFilePath ? (
          <CodeEditor
            key={editorKey}
            content={editorContent}
            onContentChange={handleContentChange}
            onSave={handleSave}
            isSaving={saveMutation.isPending}
          />
        ) : (
          <div className="flex flex-1 items-center justify-center text-muted-foreground">
            <div className="flex flex-col items-center gap-2">
              <File className="h-12 w-12 opacity-20" />
              <p className="text-sm">
                {allFilePaths.length === 0
                  ? "No files found in this directory."
                  : "Select a file to start editing."}
              </p>
            </div>
          </div>
        )}

        {/* Right: PDF preview + Tasks */}
        <div className="flex flex-col border-l w-[400px] shrink-0">
          <PdfPreview
            projectId={projectId}
            renders={renders}
            isCompiling={compileMutation.isPending}
            onCompile={handleCompile}
            isRendersLoading={rendersQuery.isLoading}
          />
          <TasksPanel projectId={projectId} />
        </div>
      </div>
    </div>
  );
}
