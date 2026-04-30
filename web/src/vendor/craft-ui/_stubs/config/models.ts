// @ts-nocheck
/* Vendor stub: @config/models — Anthropic model catalog used by FreeFormInput. */
export interface ModelDefinition {
  name: string;
  slug: string;
  contextWindow: number;
}

export const ANTHROPIC_MODELS: ModelDefinition[] = [
  { name: 'Claude Opus 4', slug: 'claude-opus-4', contextWindow: 200_000 },
  { name: 'Claude Sonnet 4', slug: 'claude-sonnet-4', contextWindow: 200_000 },
  { name: 'Claude Haiku 4', slug: 'claude-haiku-4', contextWindow: 200_000 },
];

const BY_SLUG: Record<string, ModelDefinition> = Object.fromEntries(
  ANTHROPIC_MODELS.map((m) => [m.slug, m]),
);

export function getModelShortName(slug: string): string {
  const m = BY_SLUG[slug];
  if (!m) return slug;
  return m.name.replace(/^Claude\s+/, '');
}

export function getModelDisplayName(slug: string): string {
  return BY_SLUG[slug]?.name ?? slug;
}

export function getModelContextWindow(slug: string): number {
  return BY_SLUG[slug]?.contextWindow ?? 0;
}
