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

import { Monitor, Moon, Sun } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { useTheme, type Theme } from '@/hooks/use-theme';

const ICON_MAP: Record<Theme, typeof Sun> = {
  light: Sun,
  dark: Moon,
  system: Monitor,
};

const LABEL_MAP: Record<Theme, string> = {
  light: 'Light mode',
  dark: 'Dark mode',
  system: 'System theme',
};

/** Button that cycles through light / dark / system theme modes. */
export default function ThemeToggle() {
  const { theme, toggleTheme } = useTheme();
  const Icon = ICON_MAP[theme];

  return (
    <Button
      variant="ghost"
      size="sm"
      className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      onClick={toggleTheme}
      title={LABEL_MAP[theme]}
    >
      <Icon className="h-3.5 w-3.5" />
    </Button>
  );
}
