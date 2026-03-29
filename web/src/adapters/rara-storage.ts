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

import type {
  StorageBackend,
  StorageTransaction,
  SessionMetadata,
} from "@mariozechner/pi-web-ui";

import { api, settingsApi } from "@/api/client";
import type { ChatSession } from "@/api/types";

/**
 * Bridges pi-web-ui's StorageBackend interface to rara's REST API.
 *
 * Uses an in-memory Map cache for fast synchronous-style reads, with
 * fire-and-forget writes to the rara backend for persistence. Sessions
 * and settings are pre-populated from the API during init().
 */
export class RaraStorageBackend implements StorageBackend {
  /** Two-level cache: storeName -> (key -> value) */
  private cache = new Map<string, Map<string, unknown>>();

  /** Return the sub-map for a store, creating it if absent. */
  private store(name: string): Map<string, unknown> {
    let s = this.cache.get(name);
    if (!s) {
      s = new Map();
      this.cache.set(name, s);
    }
    return s;
  }

  /**
   * Fetch sessions and settings from the rara API and seed the local cache.
   * Must be called once before the backend is used.
   */
  async init(): Promise<void> {
    const [sessions, settings] = await Promise.all([
      api.get<ChatSession[]>("/api/v1/chat/sessions?limit=100&offset=0"),
      settingsApi.list(),
    ]);

    // Populate both session stores — pi-web-ui uses "sessions" for full data
    // and "sessions-metadata" for the lightweight list (SessionListDialog reads the latter)
    const sessionsStore = this.store("sessions");
    const metaStore = this.store("sessions-metadata");
    for (const s of sessions) {
      const meta = chatSessionToMetadata(s);
      sessionsStore.set(s.key, meta);
      metaStore.set(s.key, meta);
    }

    // Populate settings store
    const settingsStore = this.store("settings");
    for (const [k, v] of Object.entries(settings)) {
      settingsStore.set(k, v);
    }

    // Ensure provider-related stores exist (rara manages providers server-side)
    this.store("providerKeys");
    this.store("customProviders");
  }

  /** Get a value by key from a specific store. Returns null if missing. */
  async get<T = unknown>(storeName: string, key: string): Promise<T | null> {
    const val = this.store(storeName).get(key);
    return (val as T) ?? null;
  }

  /**
   * Set a value for a key in a specific store.
   * Writes to the local cache immediately and syncs to the rara API
   * in the background for the "settings" store.
   */
  async set<T = unknown>(
    storeName: string,
    key: string,
    value: T,
  ): Promise<void> {
    this.store(storeName).set(key, value);

    // Keep both session stores in sync
    if (storeName === "sessions") {
      this.store("sessions-metadata").set(key, value);
    } else if (storeName === "sessions-metadata") {
      this.store("sessions").set(key, value);
    }

    // Fire-and-forget sync for settings — coerce to string for the REST API
    if (storeName === "settings") {
      settingsApi.set(key, String(value)).catch((e) => {
        console.warn("Background settings sync failed:", e);
      });
    }
  }

  /**
   * Delete a key from a specific store.
   * For the "sessions" store, also fires a background DELETE to the API.
   */
  async delete(storeName: string, key: string): Promise<void> {
    this.store(storeName).delete(key);

    // Keep both session stores in sync and fire-and-forget API deletion
    if (storeName === "sessions" || storeName === "sessions-metadata") {
      this.store("sessions").delete(key);
      this.store("sessions-metadata").delete(key);
      api
        .del(`/api/v1/chat/sessions/${encodeURIComponent(key)}`)
        .catch((e) => {
          console.warn("Background session delete failed:", e);
        });
    }
  }

  /** Get all keys from a store, optionally filtered by prefix. */
  async keys(storeName: string, prefix?: string): Promise<string[]> {
    const allKeys = Array.from(this.store(storeName).keys());
    return prefix ? allKeys.filter((k) => k.startsWith(prefix)) : allKeys;
  }

  /**
   * Get all values from a store, sorted by an index field.
   * Since we use a flat Map (no real indices), we sort in-memory by the
   * named field on each stored object.
   */
  async getAllFromIndex<T = unknown>(
    storeName: string,
    indexName: string,
    direction: "asc" | "desc" = "asc",
  ): Promise<T[]> {
    const values = Array.from(this.store(storeName).values()) as T[];
    const sorted = values.sort((a, b) => {
      const aVal = (a as Record<string, unknown>)[indexName];
      const bVal = (b as Record<string, unknown>)[indexName];
      if (aVal === bVal) return 0;
      if (aVal == null) return 1;
      if (bVal == null) return -1;
      return aVal < bVal ? -1 : 1;
    });
    return direction === "desc" ? sorted.reverse() : sorted;
  }

  /** Clear all data from a specific store. */
  async clear(storeName: string): Promise<void> {
    this.store(storeName).clear();
  }

  /** Check if a key exists in a specific store. */
  async has(storeName: string, key: string): Promise<boolean> {
    return this.store(storeName).has(key);
  }

  /**
   * Execute an operation across stores. This is a simple pass-through
   * (non-transactional) since rara is a single-user application.
   */
  async transaction<T>(
    _storeNames: string[],
    _mode: "readonly" | "readwrite",
    operation: (tx: StorageTransaction) => Promise<T>,
  ): Promise<T> {
    const tx: StorageTransaction = {
      get: <V = unknown>(store: string, key: string) =>
        this.get<V>(store, key),
      set: <V = unknown>(store: string, key: string, value: V) =>
        this.set(store, key, value),
      delete: (store: string, key: string) => this.delete(store, key),
    };
    return operation(tx);
  }

  /** Returns zeroed quota info — storage is managed server-side. */
  async getQuotaInfo(): Promise<{
    usage: number;
    quota: number;
    percent: number;
  }> {
    return { usage: 0, quota: Infinity, percent: 0 };
  }

  /** Always returns true — persistence is guaranteed by the server. */
  async requestPersistence(): Promise<boolean> {
    return true;
  }
}

/** Map a rara ChatSession to pi-web-ui SessionMetadata. */
function chatSessionToMetadata(session: ChatSession): SessionMetadata {
  return {
    id: session.key,
    title: session.title ?? "Untitled",
    createdAt: session.created_at,
    lastModified: session.updated_at,
    messageCount: session.message_count,
    usage: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 0,
      cost: {
        input: 0,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        total: 0,
      },
    },
    thinkingLevel: "off",
    preview: session.preview ?? "",
  };
}
