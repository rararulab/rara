// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/annotations/island-dismiss-policy.ts
 */
export type IslandOutsideDismissBehavior = 'back-or-close' | 'close-only'
export type IslandOutsideDismissAction = 'back' | 'close'

export interface ResolveIslandOutsideDismissActionOptions {
  isCompactView: boolean
  behavior: IslandOutsideDismissBehavior
}

export function resolveIslandOutsideDismissAction({
  isCompactView,
  behavior,
}: ResolveIslandOutsideDismissActionOptions): IslandOutsideDismissAction {
  if (behavior === 'close-only') {
    return 'close'
  }

  return isCompactView ? 'close' : 'back'
}
