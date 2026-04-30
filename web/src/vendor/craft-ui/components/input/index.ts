/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/app-shell/input/index.ts
 */
// Main components
export { InputContainer } from './InputContainer'
export { ChatInputZone } from './ChatInputZone'
export { FreeFormInput } from './FreeFormInput'
export { StructuredInput } from './StructuredInput'

// Structured input components
export { PermissionRequest } from './structured/PermissionRequest'

// Hooks
export { useAutoGrow } from './useAutoGrow'

// Types
export type {
  InputMode,
  StructuredInputType,
  StructuredInputState,
  StructuredInputData,
  StructuredResponse,
  PermissionResponse,
  AdminApprovalResponse,
} from './structured/types'
