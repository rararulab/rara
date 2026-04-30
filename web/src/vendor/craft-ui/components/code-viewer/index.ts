/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/code-viewer/index.ts
 */
/**
 * Code viewer components for syntax highlighting and diff display.
 */

export { ShikiCodeViewer, type ShikiCodeViewerProps } from './ShikiCodeViewer'
export { ShikiDiffViewer, type ShikiDiffViewerProps, getDiffStats } from './ShikiDiffViewer'
export { UnifiedDiffViewer, type UnifiedDiffViewerProps, getUnifiedDiffStats } from './UnifiedDiffViewer'
export { DiffViewerControls, type DiffViewerControlsProps } from './DiffViewerControls'
export { DiffSplitIcon, DiffUnifiedIcon, DiffBackgroundIcon } from './DiffIcons'
export { LANGUAGE_MAP, getLanguageFromPath, formatFilePath, truncateFilePath } from './language-map'
