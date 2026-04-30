// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/chat/follow-up-helpers.ts
 */
import type { AnnotationV1 } from '@craft-agent/core'

export {
  type AnnotationFollowUpState,
  asRecord,
  normalizeFollowUpText,
  getAnnotationNoteText,
  getAnnotationFollowUpState,
  isAnnotationFollowUpSent,
} from '../annotations/follow-up-state'

export function extractAnnotationSelectedText(annotation: AnnotationV1, messageContent: string): string {
  const quoteSelector = annotation.target.selectors.find(
    (selector): selector is Extract<AnnotationV1['target']['selectors'][number], { type: 'text-quote' }> => selector.type === 'text-quote'
  )
  const quoteText = quoteSelector?.exact?.trim() ?? ''
  if (quoteText.length > 0) return quoteText

  const positionSelector = annotation.target.selectors.find(
    (selector): selector is Extract<AnnotationV1['target']['selectors'][number], { type: 'text-position' }> => selector.type === 'text-position'
  )
  if (positionSelector) {
    const start = Math.max(0, Math.min(positionSelector.start, messageContent.length))
    const end = Math.max(start, Math.min(positionSelector.end, messageContent.length))
    const slice = messageContent.slice(start, end).trim()
    if (slice.length > 0) return slice
  }

  return 'Selected text'
}
