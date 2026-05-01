/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 */

// Stub for vendored craft-ui imports of `../../shared/types` (and deeper relative paths).
// The vendored Electron app exports a large set of structural types from a top-level
// `shared/types`. Rara's web app does not use those Electron-specific shapes, but the
// vendor BFS closure still references them. We expose permissive aliases so the
// vendor tree compiles and runs; nothing in the host app should import these.

export type ElectronAPI = any;
export type TransportConnectionState = any;
export type Session = any;
export type SessionFilter = any;
export type CreateSessionOptions = any;
export type ContentBadge = any;
export type FileAttachment = any;
export type LoadedSource = any;
export type LoadedSkill = any;
export type FileSearchResult = any;
export type DirectoryListingResult = any;
export type SettingsSubpage = string;
export type SourceConnectionStatus = any;
export type SetupNeeds = any;
export type LlmConnectionSetup = any;
export type LlmConnectionWithStatus = any;
export type UpdateInfo = any;
export type WindowCloseRequest = any;
export type Workspace = any;
export type PermissionMode = 'default' | 'plan' | 'auto-edit' | 'bypassPermissions' | string;
export type PermissionRequest = any;
export type CredentialRequest = any;
export type CredentialResponse = any;
export type BrowserInstanceInfo = any;
