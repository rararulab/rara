/**
 * Vendored craft-agents-oss UI primitives — entry point.
 *
 * Source: https://github.com/lukilabs/craft-agents-oss (Apache-2.0, v0.8.12).
 * Scope: atomic UI primitives + design tokens. Markdown / code-viewer /
 * overlays / chat are intentionally NOT vendored — rara already has
 * react-markdown + rehype-highlight, and chat is craft's own product
 * surface, not a primitive.
 *
 * Consume from rara code as `@/vendor/craft-ui`.
 */

export * from './components/ui';
export { cn } from './lib/utils';
export {
  setDismissibleLayerBridge,
  getDismissibleLayerBridge,
  type DismissibleLayerBridge,
  type DismissibleLayerRegistration,
  type DismissibleLayerSnapshot,
  type DismissibleLayerType,
} from './lib/dismissible-layer-bridge';
export { InlineMenuSurface, type InlineMenuSurfaceOptions } from './components/ui/InlineMenuSurface';
