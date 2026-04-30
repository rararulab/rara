/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/packages/ui/src/components/overlay/rich-block-interaction-spec.ts
 */
export interface InteractionTranslate {
  x: number
  y: number
}

export interface RichBlockInteractionState {
  scale: number
  translate: InteractionTranslate
  isDragging: boolean
  isAnimating: boolean
}

export interface RichBlockInteractionOptions {
  isOpen: boolean
  minScale?: number
  maxScale?: number
  zoomStepFactor?: number
  wheelSensitivity?: {
    mouse: number
    trackpadPinch: number
  }
  keyboardShortcuts?: boolean
}

export interface RichBlockInteractionActions {
  reset: () => void
  zoomIn: () => void
  zoomOut: () => void
  zoomToPreset: (percent: number) => void
  zoomToFit: (content: { width: number; height: number } | null) => void
}

export const RICH_BLOCK_DEFAULTS = {
  minScale: 0.25,
  maxScale: 4,
  zoomStepFactor: 1.25,
  zoomPresets: [25, 50, 75, 100, 150, 200, 400],
  wheelSensitivity: {
    mouse: 0.003,
    trackpadPinch: 0.01,
  },
} as const
