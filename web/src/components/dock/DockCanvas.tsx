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

import { useEffect, useRef } from "react";
import { LayoutDashboard } from "lucide-react";
import type { DockStore } from "@/hooks/use-dock-store";
import DockBlockRenderer from "./DockBlockRenderer";

interface DockCanvasProps {
  store: DockStore;
}

export default function DockCanvas({ store }: DockCanvasProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const prevBlockCount = useRef(store.blocks.length);

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
    <div ref={scrollRef} className="flex-1 overflow-y-auto p-4">
      <div className="mx-auto max-w-3xl space-y-3">
        {store.blocks.map((block) => (
          <DockBlockRenderer
            key={block.id}
            block={block}
            onDismissDiff={store.dismissDiff}
          />
        ))}
      </div>
    </div>
  );
}
