// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/markdown/RichBlockShell.tsx
 */
import * as React from 'react'
import { Pencil } from 'lucide-react'
import { cn } from '../../lib/utils'
import { TiptapHoverActionsHost, TiptapHoverActions, TiptapHoverActionButton } from './TiptapHoverActions'

interface RichBlockShellProps {
  children: React.ReactNode
  onEdit?: () => void
  editTitle?: string
  className?: string
}

export function RichBlockShell({ children, onEdit, editTitle = 'Edit block', className }: RichBlockShellProps) {
  return (
    <TiptapHoverActionsHost className={cn('group', className)}>
      {onEdit && (
        <TiptapHoverActions>
          <TiptapHoverActionButton
            onMouseDown={(event) => {
              // Keep focus/selection in ProseMirror so BubbleMenu anchor is stable on first open.
              event.preventDefault()
              event.stopPropagation()
            }}
            onClick={(event) => {
              event.preventDefault()
              event.stopPropagation()
              onEdit()
            }}
            className="rich-block-edit-button"
            title={editTitle}
            aria-label={editTitle}
          >
            <Pencil className="w-3.5 h-3.5" />
          </TiptapHoverActionButton>
        </TiptapHoverActions>
      )}
      {children}
    </TiptapHoverActionsHost>
  )
}
