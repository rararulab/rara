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

import { useCallback, useMemo, useState, type ReactNode } from 'react';

import SettingsModal from './SettingsModal';
import {
  SettingsModalContext,
  type SettingsModalContextValue,
  type SettingsOpenOptions,
} from './SettingsModalContext';
import type { SettingsPage } from './SettingsPanel';

/**
 * Provides a single admin-settings modal instance to the whole tree and
 * exposes imperative open/close helpers via {@link useSettingsModal}. The
 * modal itself is rendered once here so every descendant shares state —
 * mounting it per-caller would reset the active section on re-open.
 */
export function SettingsModalProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [section, setSection] = useState<SettingsPage | undefined>(undefined);
  const [options, setOptions] = useState<SettingsOpenOptions | undefined>(undefined);

  const openSettings = useCallback((next?: SettingsPage, nextOptions?: SettingsOpenOptions) => {
    setSection(next);
    setOptions(nextOptions);
    setOpen(true);
  }, []);

  const closeSettings = useCallback(() => {
    setOpen(false);
  }, []);

  const value = useMemo<SettingsModalContextValue>(
    () => ({ openSettings, closeSettings }),
    [openSettings, closeSettings],
  );

  return (
    <SettingsModalContext.Provider value={value}>
      {children}
      <SettingsModal open={open} onClose={closeSettings} section={section} options={options} />
    </SettingsModalContext.Provider>
  );
}
