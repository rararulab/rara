/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/ui/PanelHeaderCenterButton.tsx
 */
import * as React from 'react'
import { forwardRef } from 'react'
import { Tooltip, TooltipTrigger, TooltipContent } from '@craft-agent/ui'
import { cn } from '~vendor/lib/utils'

interface PanelHeaderCenterButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** Icon as React element - caller controls size/styling */
  icon: React.ReactNode
  /** Optional tooltip text */
  tooltip?: string
}

export const PanelHeaderCenterButton = forwardRef<HTMLButtonElement, PanelHeaderCenterButtonProps>(
  ({ icon, tooltip, className, ...props }, ref) => {
    const button = (
      <button
        ref={ref}
        type="button"
        aria-label={props['aria-label'] ?? tooltip}
        className={cn(
          "panel-header-btn inline-flex items-center justify-center",
          "p-1.5 shrink-0 rounded-[6px] titlebar-no-drag",
          "bg-background shadow-minimal",
          "opacity-70 hover:opacity-100",
          "transition-opacity focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
          "disabled:pointer-events-none disabled:opacity-50",
          className
        )}
        {...props}
      >
        {icon}
      </button>
    )

    if (tooltip) {
      return (
        <Tooltip>
          <TooltipTrigger asChild>{button}</TooltipTrigger>
          <TooltipContent>{tooltip}</TooltipContent>
        </Tooltip>
      )
    }

    return button
  }
)
PanelHeaderCenterButton.displayName = 'PanelHeaderCenterButton'
