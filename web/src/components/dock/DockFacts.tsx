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

import { BookOpenCheck, Plus, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';

import { Button } from '@/components/ui/button';
import type { DockStore } from '@/hooks/use-dock-store';

interface DockFactsProps {
  store: DockStore;
}

export default function DockFacts({ store }: DockFactsProps) {
  const { facts } = store;
  const [isAdding, setIsAdding] = useState(false);
  const [newContent, setNewContent] = useState('');
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState('');
  const addRef = useRef<HTMLTextAreaElement>(null);
  const editRef = useRef<HTMLTextAreaElement>(null);

  const handleAdd = useCallback(() => {
    const text = newContent.trim();
    if (text) {
      store.addFact(text);
    }
    setNewContent('');
    setIsAdding(false);
  }, [newContent, store]);

  const startEdit = useCallback((id: string, content: string) => {
    setEditingId(id);
    setEditText(content);
  }, []);

  const commitEdit = useCallback(() => {
    if (editingId && editText.trim()) {
      store.updateFact(editingId, editText.trim());
    }
    setEditingId(null);
    setEditText('');
  }, [editingId, editText, store]);

  // Focus textarea when adding/editing starts
  useEffect(() => {
    if (isAdding && addRef.current) {
      addRef.current.focus();
    }
  }, [isAdding]);

  useEffect(() => {
    if (editingId && editRef.current) {
      editRef.current.focus();
      editRef.current.select();
    }
  }, [editingId]);

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      {/* Add fact button / form */}
      <div className="shrink-0 border-b border-border/30 p-2">
        {isAdding ? (
          <div className="space-y-2">
            <textarea
              ref={addRef}
              value={newContent}
              onChange={(e) => setNewContent(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  handleAdd();
                }
                if (e.key === 'Escape') {
                  setIsAdding(false);
                  setNewContent('');
                }
              }}
              placeholder="Enter a fact..."
              className="w-full resize-none rounded border border-border/60 bg-card/60 px-2 py-1.5 text-xs placeholder:text-muted-foreground/60 focus:border-ring focus:outline-none"
              rows={3}
            />
            <div className="flex items-center gap-1.5">
              <Button
                size="sm"
                className="h-6 px-2 text-[11px]"
                onClick={handleAdd}
                disabled={!newContent.trim()}
              >
                Add
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-[11px]"
                onClick={() => {
                  setIsAdding(false);
                  setNewContent('');
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-full justify-start gap-1.5 text-xs text-muted-foreground"
            onClick={() => setIsAdding(true)}
          >
            <Plus className="h-3 w-3" />
            Add fact
          </Button>
        )}
      </div>

      {/* Facts list */}
      <div className="flex-1 overflow-y-auto">
        {facts.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 pt-16 text-center text-muted-foreground">
            <BookOpenCheck className="h-8 w-8 opacity-30" />
            <p className="text-xs">Consensus facts will accumulate here</p>
          </div>
        ) : (
          <div className="space-y-1 p-2">
            {facts.map((fact) => {
              const isEditing = editingId === fact.id;
              return (
                <div
                  key={fact.id}
                  className="group rounded-lg bg-muted/40 px-3 py-2 transition-colors hover:bg-muted/60"
                >
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
                          setEditingId(null);
                          setEditText('');
                        }
                      }}
                      onBlur={commitEdit}
                      className="w-full resize-none rounded border border-border/60 bg-card/60 px-2 py-1 text-xs focus:border-ring focus:outline-none"
                      rows={3}
                    />
                  ) : (
                    <div className="flex items-start gap-2">
                      <p
                        className="flex-1 cursor-pointer text-xs leading-relaxed text-foreground/90"
                        onClick={() => startEdit(fact.id, fact.content)}
                      >
                        {fact.content}
                      </p>
                      <button
                        className="mt-0.5 shrink-0 rounded p-0.5 text-muted-foreground/50 opacity-0 transition-all hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
                        onClick={() => store.removeFact(fact.id)}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                  )}
                  {!isEditing && (
                    <p className="mt-1 text-[10px] text-muted-foreground">
                      {fact.source === 'human' ? 'You confirmed' : 'Agent inferred'}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
