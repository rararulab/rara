/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/hooks/useResizablePanels.ts
 */
import { useState, useCallback } from 'react'
import * as storage from '@/lib/local-storage'

export function useResizablePanels(key: string, defaultSizes: number[]) {
  const [layout, setLayout] = useState<number[]>(() => {
    const saved = storage.get<number[]>(storage.KEYS.panelLayout, [], key)
    if (saved.length === defaultSizes.length) {
      return saved
    }
    return defaultSizes
  })

  const onLayoutChange = useCallback((sizes: number[]) => {
    setLayout(sizes)
    storage.set(storage.KEYS.panelLayout, sizes, key)
  }, [key])

  return { layout, onLayoutChange }
}
