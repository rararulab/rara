// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/terminal/index.ts
 */
/**
 * Terminal output components for displaying command results.
 */

export { TerminalOutput, type TerminalOutputProps, type ToolType } from './TerminalOutput'
export {
  parseAnsi,
  stripAnsi,
  isGrepContentOutput,
  parseGrepOutput,
  ANSI_COLORS,
  type AnsiSpan,
  type GrepLine,
} from './ansi-parser'
