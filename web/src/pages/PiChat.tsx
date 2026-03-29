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

import { useEffect, useRef } from "react";
import {
  AppStorage,
  setAppStorage,
  SessionsStore,
  SettingsStore,
  ProviderKeysStore,
  CustomProvidersStore,
} from "@mariozechner/pi-web-ui";
import { Agent } from "@mariozechner/pi-agent-core";
import { RaraStorageBackend } from "@/adapters/rara-storage";
import { createRaraStreamFn } from "@/adapters/rara-stream";

/**
 * Fullscreen wrapper that mounts pi-web-ui's <pi-chat-panel> Web Component,
 * wiring it up to rara's storage backend and WebSocket stream function.
 */
export default function PiChat() {
  const containerRef = useRef<HTMLDivElement>(null);
  const initRef = useRef(false);

  useEffect(() => {
    if (initRef.current || !containerRef.current) return;
    initRef.current = true;

    const container = containerRef.current;

    (async () => {
      // 1. Create and initialize the rara storage backend
      const backend = new RaraStorageBackend();
      await backend.init();

      // 2. Create store instances and wire up the backend
      const settings = new SettingsStore();
      settings.setBackend(backend);

      const providerKeys = new ProviderKeysStore();
      providerKeys.setBackend(backend);

      const sessions = new SessionsStore();
      sessions.setBackend(backend);

      const customProviders = new CustomProvidersStore();
      customProviders.setBackend(backend);

      // 3. Create AppStorage and set it as the global instance
      const storage = new AppStorage(
        settings,
        providerKeys,
        sessions,
        customProviders,
        backend,
      );
      setAppStorage(storage);

      // 4. Create the Agent with rara's WebSocket-backed stream function
      const agent = new Agent({ streamFn: createRaraStreamFn() });

      // 5. Mount the ChatPanel custom element
      const chatPanel = document.createElement("pi-chat-panel") as import("@mariozechner/pi-web-ui").ChatPanel;
      container.appendChild(chatPanel);

      // 6. Wire agent into the panel — skip API key prompt since rara manages keys server-side
      await chatPanel.setAgent(agent, {
        onApiKeyRequired: async () => true,
      });
    })();

    return () => {
      // Cleanup: remove the Web Component on unmount
      container.innerHTML = "";
    };
  }, []);

  return <div ref={containerRef} className="h-screen w-screen" />;
}
