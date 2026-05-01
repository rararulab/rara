// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/ui/skill-avatar.tsx
 */
/**
 * SkillAvatar - Thin wrapper around EntityIcon for skills.
 *
 * Sets fallbackIcon={Zap} and delegates all rendering to EntityIcon.
 * Use `fluid` prop for fill-parent sizing (e.g., Info_Page.Hero).
 */

import { Zap } from 'lucide-react'
import { EntityIcon } from '~vendor/components/electron-ui/entity-icon'
import { useEntityIcon } from '~vendor/lib/electron/icon-cache'
import type { IconSize } from '@craft-agent/shared/icons'
import type { LoadedSkill } from '../../../shared/types'

interface SkillAvatarProps {
  /** LoadedSkill object */
  skill: LoadedSkill
  /** Size variant */
  size?: IconSize
  /** Fill parent container (h-full w-full). Overrides size. */
  fluid?: boolean
  /** Additional className overrides */
  className?: string
  /** Workspace ID for loading local icons */
  workspaceId?: string
}

export function SkillAvatar({ skill, size = 'md', fluid, className, workspaceId }: SkillAvatarProps) {
  const icon = useEntityIcon({
    workspaceId: workspaceId ?? '',
    entityType: 'skill',
    identifier: skill.slug,
    iconPath: skill.iconPath,
    iconValue: skill.metadata.icon,
  })

  return (
    <EntityIcon
      icon={icon}
      size={size}
      fallbackIcon={Zap}
      alt={skill.metadata.name}
      className={className}
      containerClassName={fluid ? 'h-full w-full' : undefined}
    />
  )
}
