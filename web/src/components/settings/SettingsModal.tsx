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
import { X } from "lucide-react";
import SettingsPanel, { type SettingsPage } from "./SettingsPanel";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
  section?: SettingsPage;
}

/**
 * Floating admin-settings modal. Rendered from anywhere in the tree via
 * {@link SettingsModalProvider}. Uses a custom shell rather than the shadcn
 * `Dialog` primitive because the settings panel needs a large viewport with
 * its own internal scroll regions that `Dialog`'s capped `max-w-lg`/`grid`
 * content layout fights against.
 */
export default function SettingsModal({ open, onClose, section }: SettingsModalProps) {
  const backdropRef = useRef<HTMLDivElement>(null);

  // Escape-to-close + body scroll lock. Scoped to `open` so the listeners
  // and lock are installed exactly while the modal is visible.
  useEffect(() => {
    if (!open) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);

    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    return () => {
      window.removeEventListener("keydown", onKeyDown);
      document.body.style.overflow = prevOverflow;
    };
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      ref={backdropRef}
      className="rara-admin fixed inset-0 z-50 bg-black/40 backdrop-blur-sm"
      onClick={(e) => {
        // Close only when the mousedown originates on the backdrop itself,
        // so drags ending inside an input (value selection) don't dismiss.
        if (e.target === backdropRef.current) onClose();
      }}
    >
      <div
        className="relative mx-auto my-[5vh] flex h-[90vh] w-[min(1200px,90vw)] flex-col overflow-hidden rounded-xl border border-border/60 bg-background shadow-2xl"
      >
        <button
          type="button"
          onClick={onClose}
          aria-label="Close settings"
          className="absolute right-3 top-3 z-10 flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-secondary hover:text-foreground transition-colors cursor-pointer"
        >
          <X className="h-4 w-4" />
        </button>
        <div className="min-h-0 flex-1 overflow-hidden">
          <SettingsPanel initialSection={section} />
        </div>
      </div>
    </div>
  );
}
