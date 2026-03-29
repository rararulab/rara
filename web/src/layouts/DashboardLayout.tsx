/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { Activity, Bot, LayoutDashboard } from 'lucide-react';
import { cn } from '@/lib/utils';
import { settingsApi } from '@/api/client';
import { Button } from '@/components/ui/button';
import OnboardingModal, { isOnboardingDismissed } from '@/components/OnboardingModal';
import ThemeToggle from '@/components/ThemeToggle';

/** Routes that need zero padding in the main content area. */
const FULL_BLEED_ROUTES = new Set(['/agent', '/docs', '/dock']);

/** Routes that need full bleed when they match as a prefix. */
const FULL_BLEED_PREFIXES: string[] = [];

const SETTINGS_KEYS = {
  defaultProvider: 'llm.default_provider',
  openrouterEnabled: 'llm.providers.openrouter.enabled',
  openrouterApiKey: 'llm.providers.openrouter.api_key',
  ollamaEnabled: 'llm.providers.ollama.enabled',
  ollamaApiKey: 'llm.providers.ollama.api_key',
  ollamaBaseUrl: 'llm.providers.ollama.base_url',
  codexEnabled: 'llm.providers.codex.enabled',
} as const;

function hasConfiguredLlmProvider(settings: Record<string, string> | undefined): boolean {
  if (!settings) {
    return false;
  }

  const defaultProvider = settings[SETTINGS_KEYS.defaultProvider]?.trim();
  if (!defaultProvider) {
    return false;
  }

  switch (defaultProvider) {
    case 'openrouter':
      return (
        settings[SETTINGS_KEYS.openrouterEnabled] === 'true' &&
        Boolean(settings[SETTINGS_KEYS.openrouterApiKey]?.trim())
      );
    case 'ollama':
      return (
        settings[SETTINGS_KEYS.ollamaEnabled] === 'true' &&
        Boolean(settings[SETTINGS_KEYS.ollamaApiKey]?.trim()) &&
        Boolean(settings[SETTINGS_KEYS.ollamaBaseUrl]?.trim())
      );
    case 'codex':
      return settings[SETTINGS_KEYS.codexEnabled] === 'true';
    default:
      return false;
  }
}

export default function DashboardLayout() {
  const location = useLocation();
  const navigate = useNavigate();
  const isFullBleed = FULL_BLEED_ROUTES.has(location.pathname) || FULL_BLEED_PREFIXES.some(p => location.pathname.startsWith(p));

  const settingsQuery = useQuery({
    queryKey: ['settings'],
    queryFn: () => settingsApi.list(),
  });

  const shouldShowOnboarding = !isOnboardingDismissed();
  const [onboardingOpen, setOnboardingOpen] = useState(true);

  const handleOnboardingDismiss = () => {
    setOnboardingOpen(false);
  };

  return (
    <div className="flex h-screen bg-transparent">
      {shouldShowOnboarding && (
        <OnboardingModal
          open={onboardingOpen}
          onDismiss={handleOnboardingDismiss}
          showLlmProviderPrompt={!hasConfiguredLlmProvider(settingsQuery.data)}
        />
      )}

      <main className={cn('relative flex min-w-0 flex-1 flex-col', isFullBleed ? 'overflow-hidden' : 'overflow-auto')}>
        {/* Top bar */}
        <div className="flex shrink-0 items-center justify-end gap-2 border-b border-border/40 bg-background/30 px-4 py-1.5 backdrop-blur-sm">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => navigate('/kernel-top')}
          >
            <Activity className="h-3.5 w-3.5" />
            Kernel
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => navigate('/symphony')}
          >
            <Bot className="h-3.5 w-3.5" />
            Symphony
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => navigate('/dock')}
          >
            <LayoutDashboard className="h-3.5 w-3.5" />
            Dock
          </Button>
          <ThemeToggle />
        </div>

        <div className={cn('flex-1 min-h-0', isFullBleed ? 'p-2 md:p-3' : 'p-4 md:p-6')}>
          <Outlet />
        </div>
      </main>
    </div>
  );
}
