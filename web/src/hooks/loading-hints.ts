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

/**
 * Poetic "thinking" hints shown while the web chat timeline waits for
 * the first LLM delta. Mirrors the Telegram adapter's
 * `crates/channels/src/telegram/loading_hints.rs` so the two channels
 * feel cohesive, but lives in TS to avoid pulling a Rust string table
 * across the WASM boundary just to render eight bytes of placeholder.
 */
export const LOADING_HINTS: readonly string[] = [
  '稍候片刻，日出文自明。',
  '风过空庭，字句正徐来。',
  '纸白微明，未成篇章。',
  '夜退星沉，此页初醒。',
  '墨痕未定，片语已生香。',
  '云开一隙，文章将至。',
  '万籁俱寂，万字将成。',
  '且听风定，再看句成。',
];

/** Return a random hint from {@link LOADING_HINTS}. */
export function randomLoadingHint(): string {
  const idx = Math.floor(Math.random() * LOADING_HINTS.length);
  return LOADING_HINTS[idx] ?? LOADING_HINTS[0];
}
