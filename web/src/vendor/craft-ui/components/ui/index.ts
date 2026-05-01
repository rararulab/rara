// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/ui/index.ts
 *
 * Local modifications: removed exports for `BrowserShader`, `BrowserControls`,
 * `BrowserEmptyStateCard` — those are craft's own browser-overlay primitives
 * and not vendored. Re-add only if a downstream rara feature actually needs them.
 */

/**
 * UI primitives for vendored craft-ui.
 */

export { Spinner, type SpinnerProps, LoadingIndicator, type LoadingIndicatorProps } from './LoadingIndicator'
export {
  SimpleDropdown,
  SimpleDropdownItem,
  type SimpleDropdownProps,
  type SimpleDropdownItemProps,
} from './SimpleDropdown'
export {
  PreviewHeader,
  PreviewHeaderBadge,
  PREVIEW_BADGE_VARIANTS,
  type PreviewHeaderProps,
  type PreviewHeaderBadgeProps,
  type PreviewBadgeVariant,
} from './PreviewHeader'
export {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuSub,
  DropdownMenuShortcut,
  StyledDropdownMenuContent,
  StyledDropdownMenuItem,
  StyledDropdownMenuSeparator,
  StyledDropdownMenuSubTrigger,
  StyledDropdownMenuSubContent,
} from './StyledDropdown'
export {
  FilterableSelectPopover,
  type FilterableSelectPopoverProps,
  type FilterableSelectRenderState,
} from './FilterableSelectPopover'
export {
  Island,
  IslandContentView,
  type IslandProps,
  type IslandContentViewProps,
  type IslandTransitionConfig,
  type IslandActiveViewSize,
  type IslandMorphTarget,
  type IslandDialogBehavior,
  type AnchorX,
  type AnchorY,
} from './Island'
export {
  IslandFollowUpContentView,
  type IslandFollowUpContentViewProps,
  type IslandFollowUpMode,
} from './IslandFollowUpContentView'
export {
  useIslandNavigation,
  type IslandNavigation,
} from './useIslandNavigation'
