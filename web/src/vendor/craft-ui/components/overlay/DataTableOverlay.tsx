// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/overlay/DataTableOverlay.tsx
 */
/**
 * DataTableOverlay - Fullscreen/modal overlay for viewing data tables
 *
 * Uses PreviewOverlay as the base for consistent modal/fullscreen behavior.
 * Renders children (typically a data table) without scroll constraints,
 * allowing the full table to be visible in an expanded view.
 */

import * as React from 'react'
import type { ReactNode } from 'react'
import { Table2 } from 'lucide-react'
import { PreviewOverlay, type BadgeVariant } from './PreviewOverlay'

export interface DataTableOverlayProps {
  /** Whether the overlay is visible */
  isOpen: boolean
  /** Callback when the overlay should close */
  onClose: () => void
  /** Title for the overlay header (e.g., "Permissions", "Tools") */
  title: string
  /** Optional subtitle (e.g., row count) */
  subtitle?: string
  /** Theme mode for dark/light styling (defaults to 'light') */
  theme?: 'light' | 'dark'
  /** Badge variant for the header (default: gray) */
  badgeVariant?: BadgeVariant
  /** Actions to show in header right side (e.g., copy dropdown) */
  headerActions?: ReactNode
  /** The data table content to render */
  children: ReactNode
}

export function DataTableOverlay({
  isOpen,
  onClose,
  title,
  subtitle,
  theme,
  badgeVariant = 'gray',
  headerActions,
  children,
}: DataTableOverlayProps) {
  return (
    <PreviewOverlay
      isOpen={isOpen}
      onClose={onClose}
      theme={theme}
      typeBadge={{
        icon: Table2,
        label: 'Table',
        variant: badgeVariant,
      }}
      title={title}
      subtitle={subtitle}
      headerActions={headerActions}
    >
      {/* Table content — scrolling is handled by the parent overlay's scroll container */}
      <div>
        {children}
      </div>
    </PreviewOverlay>
  )
}
