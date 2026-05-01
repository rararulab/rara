// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/app-shell/input/useAutoGrow.ts
 */
import { useEffect, useCallback, useRef } from 'react'

interface UseAutoGrowOptions {
  /** Minimum height in pixels */
  minHeight?: number
  /** Maximum height in pixels (optional - unlimited if not set) */
  maxHeight?: number
}

/**
 * Hook to auto-grow a textarea based on content
 *
 * Usage:
 * ```tsx
 * const { ref, adjustHeight } = useAutoGrow({ minHeight: 72 })
 * <textarea ref={ref} onChange={(e) => { setValue(e.target.value); adjustHeight() }} />
 * ```
 */
export function useAutoGrow<T extends HTMLTextAreaElement>({
  minHeight = 72,
  maxHeight,
}: UseAutoGrowOptions = {}) {
  const ref = useRef<T>(null)

  const adjustHeight = useCallback(() => {
    const textarea = ref.current
    if (!textarea) return

    // Reset height to auto to get the correct scrollHeight
    textarea.style.height = 'auto'

    // Calculate new height
    let newHeight = Math.max(textarea.scrollHeight, minHeight)

    // Apply max height if set
    if (maxHeight) {
      newHeight = Math.min(newHeight, maxHeight)
    }

    textarea.style.height = `${newHeight}px`
  }, [minHeight, maxHeight])

  // Adjust on mount and when dependencies change
  useEffect(() => {
    adjustHeight()
  }, [adjustHeight])

  return { ref, adjustHeight }
}
