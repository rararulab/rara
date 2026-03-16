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

import { useCallback, useEffect, useRef, useState } from "react";
import { LayoutDashboard, Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import type { DockStore } from "@/hooks/use-dock-store";
import DockBlockRenderer from "./DockBlockRenderer";

interface DockCanvasProps {
  store: DockStore;
}

interface SelectionPopup {
  x: number;
  y: number;
  text: string;
  blockId: string;
  anchorY: number;
}

export default function DockCanvas({ store }: DockCanvasProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const prevBlockCount = useRef(store.blocks.length);
  const [selectionPopup, setSelectionPopup] = useState<SelectionPopup | null>(
    null,
  );

  // Auto-scroll when new blocks are added
  useEffect(() => {
    if (store.blocks.length > prevBlockCount.current && scrollRef.current) {
      scrollRef.current.scrollTo({
        top: scrollRef.current.scrollHeight,
        behavior: "smooth",
      });
    }
    prevBlockCount.current = store.blocks.length;
  }, [store.blocks.length]);

  // Detect text selection on mouseup
  const handleMouseUp = useCallback(() => {
    // Delay slightly so the browser finalizes the selection
    requestAnimationFrame(() => {
      const selection = window.getSelection();
      if (!selection || selection.isCollapsed || !selection.rangeCount) {
        return;
      }

      const text = selection.toString().trim();
      if (!text) return;

      const range = selection.getRangeAt(0);
      const rect = range.getBoundingClientRect();

      // Find the closest dock-block ancestor
      const node = range.startContainer;
      const el =
        node instanceof HTMLElement ? node : node.parentElement;
      const blockEl = el?.closest(".dock-block");
      if (!blockEl) return;

      const blockId = blockEl.getAttribute("data-block-id") ?? "";
      const scrollEl = scrollRef.current;
      if (!scrollEl) return;

      const scrollRect = scrollEl.getBoundingClientRect();

      setSelectionPopup({
        x: rect.left - scrollRect.left + rect.width / 2,
        y: rect.top - scrollRect.top + scrollEl.scrollTop - 8,
        text,
        blockId,
        anchorY: rect.top - scrollRect.top + scrollEl.scrollTop,
      });
    });
  }, []);

  // Dismiss popup on click outside or scroll
  useEffect(() => {
    const dismiss = () => setSelectionPopup(null);
    const scrollEl = scrollRef.current;
    if (scrollEl) {
      scrollEl.addEventListener("scroll", dismiss, { passive: true });
    }
    return () => {
      scrollEl?.removeEventListener("scroll", dismiss);
    };
  }, []);

  const handleAddNote = useCallback(() => {
    if (!selectionPopup) return;

    store.addAnnotation({
      block_id: selectionPopup.blockId,
      content: "",
      anchor_y: selectionPopup.anchorY,
      selection: {
        start: 0,
        end: selectionPopup.text.length,
        text: selectionPopup.text,
      },
    });

    // Switch to annotations tab
    store.setActiveTab("annotations");
    window.getSelection()?.removeAllRanges();
    setSelectionPopup(null);
  }, [selectionPopup, store]);

  if (store.blocks.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 text-muted-foreground">
        <LayoutDashboard className="h-12 w-12 opacity-30" />
        <div className="text-center">
          <p className="text-sm font-medium">Canvas is empty</p>
          <p className="mt-1 text-xs opacity-70">
            Send a message or use a{" "}
            <span className="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">
              ,command
            </span>{" "}
            to get started
          </p>
        </div>
      </div>
    );
  }

  return (
    <div ref={scrollRef} className="relative flex-1 overflow-y-auto p-4">
      {/* Main block content */}
      <div className="mx-auto max-w-3xl space-y-3">
        {store.blocks.map((block) => (
          <div
            key={block.id}
            data-block-id={block.id}
            className="dock-block relative"
            onMouseUp={handleMouseUp}
          >
            <DockBlockRenderer
              block={block}
              onDismissDiff={store.dismissDiff}
            />

            {/* Active annotation highlight overlay */}
            {store.activeAnnotation &&
              store.annotations
                .filter(
                  (a) =>
                    a.block_id === block.id && a.id === store.activeAnnotation,
                )
                .map((a) => (
                  <div
                    key={`highlight-${a.id}`}
                    className="pointer-events-none absolute inset-x-0 rounded-lg border-2 border-primary/30 bg-primary/5"
                    style={{ top: -2, bottom: -2 }}
                  />
                ))}
          </div>
        ))}
      </div>

      {/* Annotation margin dots */}
      {store.annotations.length > 0 && (
        <div className="absolute right-2 top-4 flex w-4 flex-col">
          {store.annotations.map((ann) => (
            <button
              key={ann.id}
              className={cn(
                "absolute right-0 h-2.5 w-2.5 rounded-full border transition-all",
                ann.id === store.activeAnnotation
                  ? "border-primary bg-primary scale-125"
                  : "border-muted-foreground/30 bg-muted-foreground/20 hover:border-primary/60 hover:bg-primary/40",
              )}
              style={{ top: ann.anchor_y }}
              onClick={() =>
                store.setActiveAnnotation(
                  ann.id === store.activeAnnotation ? null : ann.id,
                )
              }
              title={ann.content || "Annotation"}
            />
          ))}
        </div>
      )}

      {/* Selection popup: "Add note" button */}
      {selectionPopup && (
        <button
          className="absolute z-30 flex items-center gap-1 rounded-md border border-border/60 bg-card px-2 py-1 text-xs font-medium shadow-lg transition-colors hover:bg-accent"
          style={{
            left: selectionPopup.x,
            top: selectionPopup.y,
            transform: "translate(-50%, -100%)",
          }}
          onMouseDown={(e) => e.preventDefault()}
          onClick={handleAddNote}
        >
          <Plus className="h-3 w-3" />
          Add note
        </button>
      )}
    </div>
  );
}
