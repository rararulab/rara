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

import { useState, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { BrowseDirEntry } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Folder, FolderOpen, ArrowUp, Loader2, AlertCircle, Sparkles } from "lucide-react";

interface FolderPickerProps {
  open: boolean;
  onClose: () => void;
  onSelect: (path: string) => void;
  initialPath?: string;
}

export function FolderPicker({ open, onClose, onSelect, initialPath }: FolderPickerProps) {
  const [currentPath, setCurrentPath] = useState<string | undefined>(initialPath);
  const [pathInput, setPathInput] = useState("");

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["browse-directory", currentPath],
    queryFn: () => api.browseDirectory(currentPath),
    enabled: open,
    retry: false,
  });

  // Sync the input field with the resolved current_path from the API response.
  useEffect(() => {
    if (data?.current_path) {
      setPathInput(data.current_path);
    }
  }, [data?.current_path]);

  // Reset state when dialog opens.
  useEffect(() => {
    if (open) {
      setCurrentPath(initialPath);
      setPathInput(initialPath ?? "");
    }
  }, [open, initialPath]);

  function navigateTo(path: string) {
    setCurrentPath(path);
  }

  function handlePathSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pathInput.trim()) {
      navigateTo(pathInput.trim());
    }
  }

  function handleSelect() {
    if (data?.current_path) {
      onSelect(data.current_path);
      onClose();
    }
  }

  function handleEntryClick(entry: BrowseDirEntry) {
    navigateTo(entry.path);
  }

  function handleGoUp() {
    if (data?.parent_path) {
      navigateTo(data.parent_path);
    }
  }

  const errorMessage = error
    ? (error as Error).message || "Failed to browse directory"
    : null;

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Select Folder</DialogTitle>
          <DialogDescription>
            Browse the filesystem and select a directory.
          </DialogDescription>
        </DialogHeader>

        {/* Path input bar */}
        <form onSubmit={handlePathSubmit} className="flex gap-2">
          <Input
            value={pathInput}
            onChange={(e) => setPathInput(e.target.value)}
            placeholder="/path/to/directory"
            className="font-mono text-sm"
          />
          <Button type="submit" variant="outline" size="sm" className="shrink-0">
            Go
          </Button>
        </form>

        {/* Directory listing */}
        <div className="border rounded-md min-h-[300px] max-h-[400px] overflow-y-auto">
          {isLoading ? (
            <div className="flex items-center justify-center h-[300px]">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
              <span className="ml-2 text-muted-foreground text-sm">Loading...</span>
            </div>
          ) : errorMessage ? (
            <div className="flex flex-col items-center justify-center h-[300px] gap-2 px-4">
              <AlertCircle className="h-8 w-8 text-destructive/60" />
              <p className="text-sm text-destructive text-center">{errorMessage}</p>
              <Button variant="outline" size="sm" onClick={() => refetch()}>
                Retry
              </Button>
            </div>
          ) : (
            <div className="divide-y">
              {/* Go to parent */}
              {data?.parent_path && (
                <button
                  type="button"
                  className="flex items-center gap-2 w-full px-3 py-2 text-sm hover:bg-muted/50 transition-colors text-left"
                  onClick={handleGoUp}
                >
                  <ArrowUp className="h-4 w-4 text-muted-foreground shrink-0" />
                  <span className="text-muted-foreground">..</span>
                </button>
              )}

              {/* Directory entries */}
              {data?.entries.length === 0 && (
                <div className="flex items-center justify-center py-8">
                  <p className="text-sm text-muted-foreground">No subdirectories found</p>
                </div>
              )}
              {data?.entries.map((entry) => (
                <button
                  key={entry.path}
                  type="button"
                  className="flex items-center gap-2 w-full px-3 py-2 text-sm hover:bg-muted/50 transition-colors text-left group"
                  onClick={() => handleEntryClick(entry)}
                >
                  {entry.has_typ_files ? (
                    <FolderOpen className="h-4 w-4 text-amber-500 shrink-0" />
                  ) : (
                    <Folder className="h-4 w-4 text-muted-foreground shrink-0" />
                  )}
                  <span className="truncate flex-1">{entry.name}</span>
                  {entry.has_typ_files && (
                    <span className="flex items-center gap-1 text-xs text-amber-600 bg-amber-50 dark:bg-amber-950/30 px-1.5 py-0.5 rounded shrink-0">
                      <Sparkles className="h-3 w-3" />
                      .typ
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            onClick={handleSelect}
            disabled={!data?.current_path}
          >
            Select This Folder
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
