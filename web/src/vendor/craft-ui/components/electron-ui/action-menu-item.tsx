/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/ui/action-menu-item.tsx
 */
import { useActionLabel } from '~vendor/actions'
import { StyledDropdownMenuItem } from './styled-dropdown'
import type { ActionId } from '~vendor/actions/definitions'

interface ActionMenuItemProps {
  action: ActionId
  onClick?: () => void
  children?: React.ReactNode
}

export function ActionMenuItem({ action, onClick, children }: ActionMenuItemProps) {
  const { label, hotkey } = useActionLabel(action)

  return (
    <StyledDropdownMenuItem onClick={onClick}>
      <span>{children || label}</span>
      {hotkey && (
        <span className="ml-auto text-xs text-muted-foreground">{hotkey}</span>
      )}
    </StyledDropdownMenuItem>
  )
}
