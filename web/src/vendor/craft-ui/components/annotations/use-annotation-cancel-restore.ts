/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/annotations/use-annotation-cancel-restore.ts
 */
import * as React from 'react'
import type { AnchoredSelection } from './interaction-state-machine'
import { scheduleDomSelectionRestore } from './selection-restore'

export interface UseAnnotationCancelRestoreOptions<T extends HTMLElement> {
  contentRootRef: React.RefObject<T | null>
  cancelFollowUp: () => { pendingSelection: AnchoredSelection | null }
}

export function useAnnotationCancelRestore<T extends HTMLElement>({
  contentRootRef,
  cancelFollowUp,
}: UseAnnotationCancelRestoreOptions<T>) {
  return React.useCallback(() => {
    const { pendingSelection } = cancelFollowUp()
    scheduleDomSelectionRestore(contentRootRef as { current: HTMLElement | null }, pendingSelection)
  }, [cancelFollowUp, contentRootRef])
}
