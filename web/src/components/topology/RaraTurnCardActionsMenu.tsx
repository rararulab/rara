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

import { ArrowUpRight, MoreHorizontal } from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useEffect, useRef, useState } from 'react';

/**
 * Rara-side replacement for the vendor `TurnCardActionsMenu`. The vendor
 * version wraps its dropdown in `SimpleDropdown`, whose `setItemRef`
 * callback synchronously calls `setHighlightedId` while a child item is
 * mounting — that is a "setState during render of a different component"
 * pattern that React 18's strict checks turn into a console error and
 * (more relevantly) blocks the dropdown from ever opening on a real
 * browser. See issue 2032.
 *
 * Editing vendor files is forbidden, so we plug a hand-rolled popover
 * into vendor `TurnCard` via `renderActionsMenu`. The popover is
 * deliberately small: a click-toggled trigger, an outside-click /
 * Escape handler, and a single "View turn details" item. Keep it that
 * way — anything more elaborate belongs back in a real menu primitive.
 *
 * Rendering a `MoreHorizontal` lucide icon on the trigger is load-bearing:
 * the existing browser-smoke probe and the BDD scenario both look for an
 * `svg.lucide-more-horizontal` (or `lucide-ellipsis`, the new lucide
 * alias) under the hovered turn header.
 */
export interface RaraTurnCardActionsMenuProps {
  /** Open the per-turn execution-trace modal. */
  onOpenDetails: () => void;
}

export function RaraTurnCardActionsMenu({ onOpenDetails }: RaraTurnCardActionsMenuProps) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    // Close on outside click + Escape. We attach on `mousedown` rather
    // than `click` so a click on the trigger that is about to toggle
    // open does not immediately bubble back as an "outside click" close.
    const onPointerDown = (event: MouseEvent) => {
      const node = containerRef.current;
      if (!node) return;
      if (node.contains(event.target as Node)) return;
      setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    window.addEventListener('mousedown', onPointerDown);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', onPointerDown);
      window.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    // 40x40 hit area (#16): the visible chip stays small, but the
    // surrounding flex catches the click. `-m-2` reclaims layout space so
    // the wrapper does not push the turn header.
    <div ref={containerRef} className="relative -m-2 shrink-0">
      <div
        role="button"
        tabIndex={0}
        aria-haspopup="menu"
        aria-expanded={open}
        className={
          // Explicit transition list (no `transition: all`, #14) and
          // press-scale (#12, capped at 0.96).
          'flex h-10 w-10 items-center justify-center rounded-md ' +
          'transition-[opacity,transform] active:scale-[0.96] ' +
          'text-muted-foreground/50 hover:text-foreground ' +
          'opacity-0 group-hover:opacity-100 ' +
          'focus:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:opacity-100 ' +
          (open ? 'opacity-100 text-foreground' : '')
        }
        onClick={(event) => {
          event.stopPropagation();
          setOpen((prev) => !prev);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            event.stopPropagation();
            setOpen((prev) => !prev);
          }
        }}
      >
        {/*
         * Inner chip: `rounded-md` (6px) inside the surrounding 40px hit
         * region keeps the visible affordance compact while the click
         * target is generous. Shadow over border (#3) for depth.
         */}
        <span className="flex items-center justify-center rounded-md bg-background p-1 shadow-minimal">
          <MoreHorizontal className="w-3 h-3" />
        </span>
      </div>
      {/*
       * Animated open/close — spring (bounce 0) on opacity + a small
       * translateY for a subtle exit per principle #6. `initial={false}`
       * is fine here because the menu only ever animates after a user
       * click (it is never present on first paint).
       */}
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            role="menu"
            tabIndex={-1}
            initial={{ opacity: 0, y: -2 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -2 }}
            transition={{ type: 'spring', duration: 0.3, bounce: 0 }}
            className={
              'absolute right-0 top-full mt-1 z-50 min-w-[12rem] ' +
              'rounded-md border border-border bg-popover text-popover-foreground shadow-md py-1'
            }
            // Stop the parent vendor `<button>` (which wraps the header) from
            // toggling its expanded state when the user clicks an item.
            onClick={(event) => event.stopPropagation()}
          >
            {/*
             * Use `<div role="menuitem">` rather than `<button>` here:
             * vendor wraps the entire turn header in its own `<button>`
             * (TurnCard.tsx ~line 2940), so a nested `<button>` would
             * trip React's hydration warning on "button cannot be a
             * descendant of button". `tabIndex={0}` keeps it keyboard-
             * focusable; `Enter`/`Space` invoke the same action.
             */}
            <div
              role="menuitem"
              tabIndex={0}
              className={
                'w-full flex items-center gap-2 px-3 py-1.5 text-sm text-left cursor-pointer ' +
                'hover:bg-accent hover:text-accent-foreground'
              }
              onClick={(event) => {
                event.stopPropagation();
                setOpen(false);
                onOpenDetails();
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  event.stopPropagation();
                  setOpen(false);
                  onOpenDetails();
                }
              }}
            >
              <ArrowUpRight className="w-3.5 h-3.5" />
              <span>View turn details</span>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
