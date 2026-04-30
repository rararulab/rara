/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/icons/types.ts
 */
import type { SVGProps } from 'react'

export interface IconProps extends SVGProps<SVGSVGElement> {
  /**
   * Icon size. Only used when className doesn't include size classes.
   * For Tailwind, prefer using className="size-4" etc.
   */
  size?: number | string
}
