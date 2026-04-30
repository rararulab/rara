/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/annotations/annotation-host-config.ts
 */
export type AnnotationHost = 'turncard' | 'fullscreen'

export interface AnnotationCanAnnotateOptions {
  hasAddAnnotationHandler: boolean
  hasMessageId: boolean
  isStreaming: boolean
}

export function canAnnotateMessage({
  hasAddAnnotationHandler,
  hasMessageId,
  isStreaming,
}: AnnotationCanAnnotateOptions): boolean {
  return hasAddAnnotationHandler && hasMessageId && !isStreaming
}

/**
 * Portal strategy is centralized so host-specific differences are explicit.
 * Fullscreen keeps in-overlay rendering to avoid stack/clip issues with modal hosts.
 */
export function shouldRenderAnnotationIslandInPortal(host: AnnotationHost): boolean {
  return host !== 'fullscreen'
}
