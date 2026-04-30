/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/ui/action-tooltip.tsx
 */
import { useActionLabel } from '@/actions'
import { Tooltip, TooltipTrigger, TooltipContent } from '@craft-agent/ui'
import type { ActionId } from '@/actions/definitions'

interface ActionTooltipProps {
  action: ActionId
  children: React.ReactNode
}

export function ActionTooltip({ action, children }: ActionTooltipProps) {
  const { label, hotkey } = useActionLabel(action)

  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent>
        {label}
        {hotkey && <kbd className="ml-2 text-xs opacity-60">{hotkey}</kbd>}
      </TooltipContent>
    </Tooltip>
  )
}
