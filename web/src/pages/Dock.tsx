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

import { useCallback, useEffect, useState } from 'react';

import DockCanvas from '@/components/dock/DockCanvas';
import DockConsole from '@/components/dock/DockConsole';
import DockHeader from '@/components/dock/DockHeader';
import DockSidebar from '@/components/dock/DockSidebar';
import { useDockStore } from '@/hooks/use-dock-store';

export default function Dock() {
  const store = useDockStore();
  const [rightPanelOpen, setRightPanelOpen] = useState(false);

  useEffect(() => {
    void store.bootstrap();
    // Run once on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleRightPanel = useCallback(() => {
    setRightPanelOpen((prev) => !prev);
  }, []);

  return (
    <div className="app-surface flex h-full flex-col overflow-hidden rounded-2xl border border-border/60">
      <DockHeader
        store={store}
        rightPanelOpen={rightPanelOpen}
        onToggleRightPanel={toggleRightPanel}
      />

      <div className="flex min-h-0 flex-1">
        {/* Main canvas area */}
        <div className="flex min-w-0 flex-1 flex-col">
          <DockCanvas store={store} />
          <DockConsole store={store} />
        </div>

        {/* Right sidebar */}
        {rightPanelOpen && <DockSidebar store={store} />}
      </div>
    </div>
  );
}
