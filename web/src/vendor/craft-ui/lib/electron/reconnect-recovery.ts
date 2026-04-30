// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/lib/reconnect-recovery.ts
 */
import type { SessionMeta } from '~vendor/atoms/sessions'

export function getSessionsToRefreshAfterStaleReconnect(
  metaMap: Map<string, SessionMeta>,
  activeSessionId: string | null
): string[] {
  const refreshIds = new Set<string>()

  if (activeSessionId) {
    refreshIds.add(activeSessionId)
  }

  for (const [sessionId, meta] of metaMap) {
    if (meta.isProcessing) {
      refreshIds.add(sessionId)
    }
  }

  return [...refreshIds]
}
