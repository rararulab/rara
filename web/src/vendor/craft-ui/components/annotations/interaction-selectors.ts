// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/annotations/interaction-selectors.ts
 */
import type { AnnotationInteractionState } from './interaction-state-machine'

export function getAnnotationInteractionSourceKey(state: AnnotationInteractionState, messageId?: string): string {
  const messageScope = messageId ?? 'no-message'

  if (state.pendingSelection) {
    return `selection:${messageScope}:${state.pendingSelection.start}:${state.pendingSelection.end}`
  }

  if (state.activeAnnotationDetail) {
    return `annotation:${messageScope}:${state.activeAnnotationDetail.annotationId}`
  }

  return `none:${messageScope}`
}

export function getAnnotationInteractionAnchor(state: AnnotationInteractionState): { x: number; y: number } | null {
  return state.selectionMenuAnchor
}

export function hasAnnotationInteraction(state: AnnotationInteractionState): boolean {
  return Boolean(state.pendingSelection || state.activeAnnotationDetail)
}
