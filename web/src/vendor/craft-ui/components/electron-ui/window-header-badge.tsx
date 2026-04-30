// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/ui/window-header-badge.tsx
 */
/**
 * Re-export PreviewHeader components from @craft-agent/ui
 *
 * This provides backwards compatibility for existing Electron components.
 * The actual implementation is now in the shared UI package.
 */

export {
  PreviewHeader as WindowHeader,
  PreviewHeaderBadge as WindowHeaderBadge,
  PREVIEW_BADGE_VARIANTS as BADGE_VARIANTS,
  type PreviewHeaderProps as WindowHeaderProps,
  type PreviewHeaderBadgeProps as WindowHeaderBadgeProps,
  type PreviewBadgeVariant as BadgeVariant,
} from '@craft-agent/ui'
