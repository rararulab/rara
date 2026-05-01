// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/markdown/math-options.ts
 */
/**
 * Shared remark-math configuration for markdown rendering.
 *
 * We intentionally disable single-dollar inline math so currency strings
 * (e.g. $100, $2M–$4M) remain plain text.
 */
export const MARKDOWN_MATH_OPTIONS = {
  singleDollarTextMath: false,
} as const
