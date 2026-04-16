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

import type { ReactNode } from "react";

export interface MetadataChipProps {
  /** Optional leading icon (typically a lucide-react icon at h-3 w-3). */
  icon?: ReactNode;
  children: ReactNode;
}

/**
 * Compact information chip used in session headers and stat bars.
 *
 * Visual contract: rounded-md, bordered, `bg-muted/50`, 11px text.
 * Intentionally low-weight so rows of 5–8 chips stay visually quiet.
 */
export function MetadataChip({ icon, children }: MetadataChipProps) {
  return (
    <span className="inline-flex items-center gap-1 rounded-md border bg-muted/50 px-2 py-0.5 text-[11px] text-muted-foreground">
      {icon}
      {children}
    </span>
  );
}
