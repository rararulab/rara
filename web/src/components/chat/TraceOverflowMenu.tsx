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

import { useEffect, useState } from 'react';

import {
  DropdownMenu,
  DropdownMenuAnchor,
  DropdownMenuContent,
  DropdownMenuItem,
} from '@/components/ui/dropdown-menu';

/**
 * Class applied by the Lit assistant-message renderer to its overflow
 * trigger button. Document-level click delegation here keeps the Lit
 * template trivial (one button) while the actual menu lives in React.
 */
export const TRACE_OVERFLOW_TRIGGER_CLASS = 'rara-trace-overflow';

/**
 * Names of the per-turn trace events the menu items dispatch. Kept in
 * sync with the listeners in `PiChat.tsx` — the overflow menu is purely
 * a relay; clicking an item refires the same CustomEvent the legacy
 * inline buttons used to fire directly.
 */
export const EXECUTION_TRACE_EVENT = 'rara:execution-trace';
export const CASCADE_TRACE_EVENT = 'rara:cascade-trace';

interface AnchorState {
  seq: number;
  rect: DOMRect;
}

/**
 * Floating overflow menu shared by every assistant turn's `…` trigger.
 * Listens for clicks on `.rara-trace-overflow` buttons (rendered by the
 * Lit assistant-message renderer in `PiChat.tsx`), reads `data-seq` from
 * the trigger, and pops a Radix dropdown anchored to the trigger's
 * bounding rect. Selecting an item re-dispatches the original
 * `rara:execution-trace` / `rara:cascade-trace` CustomEvent so the
 * existing modal-fetch wiring in `PiChat` runs unchanged.
 */
export function TraceOverflowMenu(): React.ReactElement {
  const [anchor, setAnchor] = useState<AnchorState | null>(null);

  useEffect(() => {
    const handler = (event: MouseEvent) => {
      const target = event.target as HTMLElement | null;
      if (!target) return;
      const trigger = target.closest<HTMLButtonElement>(`.${TRACE_OVERFLOW_TRIGGER_CLASS}`);
      if (!trigger) return;
      event.preventDefault();
      event.stopPropagation();
      const seqAttr = trigger.dataset.seq;
      if (seqAttr === undefined) return;
      const seq = Number.parseInt(seqAttr, 10);
      if (!Number.isFinite(seq)) return;
      setAnchor({ seq, rect: trigger.getBoundingClientRect() });
    };
    document.addEventListener('click', handler, true);
    return () => document.removeEventListener('click', handler, true);
  }, []);

  const dispatch = (eventName: string) => {
    if (!anchor) return;
    document.dispatchEvent(
      new CustomEvent<{ seq: number }>(eventName, {
        detail: { seq: anchor.seq },
        bubbles: true,
      }),
    );
    setAnchor(null);
  };

  return (
    <DropdownMenu open={anchor !== null} onOpenChange={(o) => !o && setAnchor(null)}>
      <DropdownMenuAnchor asChild>
        <div
          aria-hidden
          style={{
            position: 'fixed',
            top: anchor?.rect.top ?? 0,
            left: anchor?.rect.left ?? 0,
            width: anchor?.rect.width ?? 0,
            height: anchor?.rect.height ?? 0,
            pointerEvents: 'none',
          }}
        />
      </DropdownMenuAnchor>
      <DropdownMenuContent align="start" sideOffset={6}>
        <DropdownMenuItem onSelect={() => dispatch(EXECUTION_TRACE_EVENT)}>
          <span aria-hidden>📊</span>
          <span>详情</span>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => dispatch(CASCADE_TRACE_EVENT)}>
          <span aria-hidden>🔍</span>
          <span>Cascade</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
