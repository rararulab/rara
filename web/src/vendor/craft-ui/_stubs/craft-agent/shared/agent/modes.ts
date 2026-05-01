// @ts-nocheck
/* Vendor stub: @craft-agent/shared/agent/modes */
export type PermissionMode = 'default' | 'plan' | 'auto-edit' | 'bypassPermissions';

export const PERMISSION_MODE_ORDER: PermissionMode[] = [
  'default',
  'plan',
  'auto-edit',
  'bypassPermissions',
];

export const PERMISSION_MODE_CONFIG: Record<PermissionMode, { label: string; description: string }> = {
  default: { label: 'Default', description: 'Ask before edits' },
  plan: { label: 'Plan', description: 'Plan only, no edits' },
  'auto-edit': { label: 'Auto-edit', description: 'Auto-approve edits' },
  bypassPermissions: { label: 'Bypass', description: 'Bypass all checks' },
};
