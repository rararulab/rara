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

import { MessageSquareDashed, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';

import type { DockStore } from '@/hooks/use-dock-store';
import { cn } from '@/lib/utils';

interface DockAnnotationsProps {
  store: DockStore;
}

export default function DockAnnotations({ store }: DockAnnotationsProps) {
  const { annotations, activeAnnotation } = store;
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState('');
  const editRef = useRef<HTMLTextAreaElement>(null);

  const sorted = [...annotations].sort((a, b) => a.anchor_y - b.anchor_y);

  const startEdit = useCallback(
    (id: string, content: string) => {
      if (activeAnnotation === id) {
        setEditingId(id);
        setEditText(content);
      }
    },
    [activeAnnotation],
  );

  const commitEdit = useCallback(() => {
    if (editingId && editText.trim()) {
      store.updateAnnotation(editingId, editText.trim());
    }
    setEditingId(null);
    setEditText('');
  }, [editingId, editText, store]);

  const cancelEdit = useCallback(() => {
    setEditingId(null);
    setEditText('');
  }, []);

  // Focus textarea when editing starts
  useEffect(() => {
    if (editingId && editRef.current) {
      editRef.current.focus();
      editRef.current.select();
    }
  }, [editingId]);

  if (sorted.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
        <MessageSquareDashed className="h-8 w-8 opacity-30" />
        <p className="text-xs">Select content to annotate</p>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {sorted.map((ann) => {
        const isActive = activeAnnotation === ann.id;
        const isEditing = editingId === ann.id;

        return (
          <div
            key={ann.id}
            className={cn(
              'group cursor-pointer border-b border-border/30 px-3 py-2.5 transition-colors',
              isActive ? 'bg-accent/40' : 'hover:bg-accent/20',
            )}
            onClick={() => store.setActiveAnnotation(isActive ? null : ann.id)}
          >
            <div className="flex items-start gap-2">
              {/* Dot indicator */}
              <div
                className={cn(
                  'mt-1.5 h-2 w-2 shrink-0 rounded-full transition-colors',
                  isActive ? 'bg-primary' : 'bg-muted-foreground/40',
                )}
              />

              <div className="min-w-0 flex-1">
                {isEditing ? (
                  <textarea
                    ref={editRef}
                    value={editText}
                    onChange={(e) => setEditText(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' && !e.shiftKey) {
                        e.preventDefault();
                        commitEdit();
                      }
                      if (e.key === 'Escape') {
                        cancelEdit();
                      }
                    }}
                    onBlur={commitEdit}
                    className="w-full resize-none rounded border border-border/60 bg-card/60 px-2 py-1 text-xs focus:border-ring focus:outline-none"
                    rows={3}
                  />
                ) : (
                  <p
                    className={cn(
                      'text-xs leading-relaxed',
                      isActive ? 'text-foreground' : 'text-foreground/80 line-clamp-2',
                    )}
                    onClick={(e) => {
                      if (isActive) {
                        e.stopPropagation();
                        startEdit(ann.id, ann.content);
                      }
                    }}
                  >
                    {ann.content}
                  </p>
                )}

                {/* Expanded details when active */}
                {isActive && !isEditing && (
                  <div className="mt-2 space-y-1.5">
                    {ann.selection?.text && (
                      <div className="rounded bg-muted/50 px-2 py-1.5 text-[11px] italic text-muted-foreground line-clamp-3">
                        &ldquo;{ann.selection.text}&rdquo;
                      </div>
                    )}
                    <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
                      <span className="capitalize">{ann.author}</span>
                      <span>&middot;</span>
                      <span>{store.formatTime(ann.timestamp)}</span>
                    </div>
                  </div>
                )}
              </div>

              {/* Delete button */}
              <button
                className={cn(
                  'mt-0.5 shrink-0 rounded p-0.5 text-muted-foreground/50 transition-colors hover:bg-destructive/10 hover:text-destructive',
                  isActive ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
                )}
                onClick={(e) => {
                  e.stopPropagation();
                  store.removeAnnotation(ann.id);
                }}
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
}
