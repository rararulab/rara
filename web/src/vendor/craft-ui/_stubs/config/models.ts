// @ts-nocheck
/* Vendor stub: @config/models — emptied for rara so the model picker never
 * flashes upstream's hardcoded Claude entries. The real catalog comes from
 * `useChatModels()` (GET /api/v1/chat/models) wired through
 * AppShellContext.llmConnections in TimelineView; this fallback only fires
 * when the context is missing entirely. Helper functions keep their
 * fallback-to-id behavior, so unknown ids render as their own slug. */
export interface ModelDefinition {
  name: string;
  slug: string;
  contextWindow: number;
}

export const ANTHROPIC_MODELS: ModelDefinition[] = [];

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
