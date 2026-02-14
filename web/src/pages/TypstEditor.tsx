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
import type { TypstProject, TypstFile, RenderResult } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import {
  ArrowLeft,
  File,
  FilePlus,
  Loader2,
  Play,
  Trash2,
  History,
  Download,
  Save,
  ChevronRight,
} from "lucide-react";

// ---------------------------------------------------------------------------
// File Tree (left column)
// ---------------------------------------------------------------------------

function FileTree({
  files,
  activeFilePath,
  onSelect,
  onCreateFile,
  onDeleteFile,
  isLoading,
}: {
  files: TypstFile[];
  activeFilePath: string | null;
  onSelect: (path: string) => void;
  onCreateFile: () => void;
  onDeleteFile: (path: string) => void;
  isLoading: boolean;
}) {
  return (
    <div className="flex flex-col border-r bg-card w-56 shrink-0">
      <div className="flex items-center justify-between border-b px-3 py-2">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Files
        </h3>
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6"
          onClick={onCreateFile}
          title="New file"
        >
          <FilePlus className="h-3.5 w-3.5" />
        </Button>
      </div>
      <div className="flex-1 overflow-y-auto p-1">
        {isLoading ? (
          <div className="space-y-1 p-1">
            {Array.from({ length: 3 }).map((_, i) => (
              <Skeleton key={i} className="h-7 w-full" />
            ))}
          </div>
        ) : files.length === 0 ? (
          <div className="p-3 text-center text-xs text-muted-foreground">
            No files yet.
          </div>
        ) : (
          <div className="space-y-0.5">
            {files.map((file) => (
              <div
                key={file.id}
                className={cn(
                  "group flex items-center gap-1.5 rounded-md px-2 py-1.5 text-sm cursor-pointer transition-colors",
                  activeFilePath === file.path
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground"
                )}
                onClick={() => onSelect(file.path)}
              >
                <File className="h-3.5 w-3.5 shrink-0" />
                <span className="flex-1 truncate font-mono text-xs">
                  {file.path}
                </span>
                <button
                  type="button"
                  className="hidden shrink-0 rounded p-0.5 text-muted-foreground hover:text-destructive group-hover:block"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDeleteFile(file.path);
                  }}
                  title="Delete file"
                >
                  <Trash2 className="h-3 w-3" />
                </button>
              </div>
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

  // Keep refs up-to-date
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
    // Only re-create the editor when the content identity changes (i.e., when switching files)
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

  // Determine which render to display: selected or latest
  const displayRenderId = selectedRenderId ?? (renders.length > 0 ? renders[0].id : null);

  // Fetch PDF blob when render changes
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

  // Clean up blob URL on unmount
  useEffect(() => {
    return () => {
      if (pdfUrl) URL.revokeObjectURL(pdfUrl);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Reset selected render when renders list changes (new compile completed)
  useEffect(() => {
    if (renders.length > 0 && selectedRenderId) {
      // If the selected render is not in the list anymore, reset
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
    <div className="flex flex-col border-l w-[400px] shrink-0">
      {/* Controls */}
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

      {/* Render history panel */}
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

      {/* PDF viewer */}
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
// TypstEditor (main page)
// ---------------------------------------------------------------------------

export default function TypstEditor() {
  const { projectId } = useParams<{ projectId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [activeFilePath, setActiveFilePath] = useState<string | null>(null);
  const [editorContent, setEditorContent] = useState<string>("");
  const [editorKey, setEditorKey] = useState(0);
  const [newFileDialogOpen, setNewFileDialogOpen] = useState(false);
  const [newFilePath, setNewFilePath] = useState("");
  const [deleteFilePath, setDeleteFilePath] = useState<string | null>(null);

  // Debounce auto-save timer
  const autoSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingContentRef = useRef<string | null>(null);

  // Fetch project details
  const projectQuery = useQuery({
    queryKey: ["typst-project", projectId],
    queryFn: () => api.get<TypstProject>(`/api/v1/typst/projects/${projectId}`),
    enabled: !!projectId,
  });

  // Fetch project files
  const filesQuery = useQuery({
    queryKey: ["typst-files", projectId],
    queryFn: () =>
      api.get<TypstFile[]>(`/api/v1/typst/projects/${projectId}/files`),
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

  const files = filesQuery.data ?? [];
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
    if (files.length > 0 && !activeFilePath) {
      const mainFile = projectQuery.data?.main_file ?? "main.typ";
      const found = files.find((f) => f.path === mainFile);
      if (found) {
        setActiveFilePath(found.path);
        setEditorContent(found.content);
        setEditorKey((k) => k + 1);
      } else {
        setActiveFilePath(files[0].path);
        setEditorContent(files[0].content);
        setEditorKey((k) => k + 1);
      }
    }
  }, [files, activeFilePath, projectQuery.data?.main_file]);

  // Save file mutation
  const saveMutation = useMutation({
    mutationFn: ({ path, content }: { path: string; content: string }) =>
      api.put<TypstFile>(
        `/api/v1/typst/projects/${projectId}/files/${encodeURIComponent(path)}`,
        { content }
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["typst-files", projectId] });
    },
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

  // Create file mutation
  const createFileMutation = useMutation({
    mutationFn: (data: { path: string; content: string }) =>
      api.post<TypstFile>(
        `/api/v1/typst/projects/${projectId}/files`,
        data
      ),
    onSuccess: (file) => {
      queryClient.invalidateQueries({ queryKey: ["typst-files", projectId] });
      setNewFileDialogOpen(false);
      setNewFilePath("");
      setActiveFilePath(file.path);
      setEditorContent(file.content);
      setEditorKey((k) => k + 1);
    },
  });

  // Delete file mutation
  const deleteFileMutation = useMutation({
    mutationFn: (path: string) =>
      api.del(
        `/api/v1/typst/projects/${projectId}/files/${encodeURIComponent(path)}`
      ),
    onSuccess: (_data, deletedPath) => {
      queryClient.invalidateQueries({ queryKey: ["typst-files", projectId] });
      setDeleteFilePath(null);
      if (activeFilePath === deletedPath) {
        setActiveFilePath(null);
        setEditorContent("");
        setEditorKey((k) => k + 1);
      }
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

      const file = files.find((f) => f.path === path);
      if (file) {
        setActiveFilePath(path);
        setEditorContent(file.content);
        setEditorKey((k) => k + 1);
      }
    },
    [activeFilePath, files, saveMutation]
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

  // Handle new file creation
  function handleCreateFile(e: React.FormEvent) {
    e.preventDefault();
    const path = newFilePath.trim();
    if (!path) return;
    const finalPath = path.endsWith(".typ") ? path : `${path}.typ`;
    createFileMutation.mutate({ path: finalPath, content: "" });
  }

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
        {projectQuery.data?.description && (
          <span className="text-xs text-muted-foreground truncate hidden md:inline">
            {projectQuery.data.description}
          </span>
        )}
      </div>

      {/* Three-column layout */}
      <div className="flex flex-1 min-h-0">
        {/* Left: File tree */}
        <FileTree
          files={files}
          activeFilePath={activeFilePath}
          onSelect={handleSelectFile}
          onCreateFile={() => setNewFileDialogOpen(true)}
          onDeleteFile={(path) => setDeleteFilePath(path)}
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
                {files.length === 0
                  ? 'No files yet. Click "+" to create one.'
                  : "Select a file to start editing."}
              </p>
            </div>
          </div>
        )}

        {/* Right: PDF preview */}
        <PdfPreview
          projectId={projectId}
          renders={renders}
          isCompiling={compileMutation.isPending}
          onCompile={handleCompile}
          isRendersLoading={rendersQuery.isLoading}
        />
      </div>

      {/* New File Dialog */}
      <Dialog open={newFileDialogOpen} onOpenChange={setNewFileDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New File</DialogTitle>
            <DialogDescription>
              Enter a file path for the new Typst file. The .typ extension will
              be added automatically if not provided.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreateFile} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="file-path">File Path</Label>
              <Input
                id="file-path"
                value={newFilePath}
                onChange={(e) => setNewFilePath(e.target.value)}
                placeholder="e.g. style.typ or template/header.typ"
                required
              />
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setNewFileDialogOpen(false);
                  setNewFilePath("");
                }}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={
                  createFileMutation.isPending || !newFilePath.trim()
                }
              >
                {createFileMutation.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete File Confirmation Dialog */}
      <Dialog
        open={deleteFilePath !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteFilePath(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete File</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{deleteFilePath}&quot;? This
              action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDeleteFilePath(null)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={deleteFileMutation.isPending}
              onClick={() => {
                if (deleteFilePath)
                  deleteFileMutation.mutate(deleteFilePath);
              }}
            >
              {deleteFileMutation.isPending ? "Deleting..." : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
