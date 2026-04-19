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

import { Sparkles } from 'lucide-react';

import { useSettingsModal } from '@/components/settings/SettingsModalProvider';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

/** localStorage key — marks the user has dismissed onboarding */
const ONBOARDING_DISMISSED_KEY = 'onboarding_dismissed';

export function isOnboardingDismissed(): boolean {
  return localStorage.getItem(ONBOARDING_DISMISSED_KEY) === 'true';
}

export function dismissOnboarding(): void {
  localStorage.setItem(ONBOARDING_DISMISSED_KEY, 'true');
}

interface OnboardingModalProps {
  open: boolean;
  onDismiss: () => void;
  showLlmProviderPrompt?: boolean;
}

export default function OnboardingModal({
  open,
  onDismiss,
  showLlmProviderPrompt = false,
}: OnboardingModalProps) {
  const { openSettings } = useSettingsModal();

  const handleSkip = () => {
    dismissOnboarding();
    onDismiss();
  };

  const handleOpenProviderSettings = () => {
    dismissOnboarding();
    onDismiss();
    openSettings('providers');
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(isOpen) => {
        if (!isOpen) handleSkip();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="h-5 w-5 text-primary" />
            Welcome to Rara!
          </DialogTitle>
          <DialogDescription>
            {showLlmProviderPrompt
              ? 'Configure an LLM provider to get started with AI features.'
              : "You're all set. Start chatting with your agent!"}
          </DialogDescription>
        </DialogHeader>

        {showLlmProviderPrompt && (
          <div className="space-y-3 py-2">
            <p className="text-sm text-muted-foreground">
              Before using AI features, you need to configure at least one LLM provider in Settings.
            </p>
            <Button onClick={handleOpenProviderSettings} className="w-full">
              Configure Provider
            </Button>
          </div>
        )}

        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={handleSkip}>
            {showLlmProviderPrompt ? 'Skip for now' : 'Got it'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
