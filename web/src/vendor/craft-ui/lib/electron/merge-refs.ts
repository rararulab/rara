/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/lib/merge-refs.ts
 */
import { type MutableRefObject, type RefCallback } from 'react'

type Ref<T> = RefCallback<T> | MutableRefObject<T> | null | undefined

/**
 * Merges multiple refs into a single ref callback.
 * Useful when an element needs to satisfy multiple ref requirements
 * (e.g., focus zone ref + hotkey scope ref).
 */
export function mergeRefs<T>(...refs: Ref<T>[]): RefCallback<T> {
  return (value: T) => {
    refs.forEach(ref => {
      if (typeof ref === 'function') {
        ref(value)
      } else if (ref != null) {
        ref.current = value
      }
    })
  }
}
