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
 * Redact secret-like fields in tool input / output before display.
 *
 * Scope: only the agent-live card / transcript dialog needs this. The
 * main chat transcript is rendered by pi-web-ui, which already has its
 * own secret-handling conventions.
 *
 * Key-based masking only — we never scan values with regex, which tends
 * to false-positive on harmless text. If you need to add a new secret
 * shape, append it to {@link SECRET_KEY_RE}.
 */

/** Field names whose values should be masked before display. */
const SECRET_KEY_RE = /(api[_-]?key|token|password|secret|authorization|bearer|private[_-]?key)/i;

/** Placeholder used to replace redacted values in rendered output. */
export const REDACTED = '\u2022\u2022\u2022\u2022\u2022\u2022';

/**
 * Walk a plain object/array and replace string/number values whose key
 * matches {@link SECRET_KEY_RE} with {@link REDACTED}. Non-string values
 * (nested objects, arrays) are recursed into; unknown branches are left
 * untouched so structural typing downstream still sees the original
 * shape.
 */
export function redactObject(input: unknown): unknown {
  if (Array.isArray(input)) {
    return input.map((v) => redactObject(v));
  }
  if (input && typeof input === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(input as Record<string, unknown>)) {
      if (SECRET_KEY_RE.test(k) && (typeof v === 'string' || typeof v === 'number')) {
        out[k] = REDACTED;
      } else {
        out[k] = redactObject(v);
      }
    }
    return out;
  }
  return input;
}

/**
 * Render a tool-argument object as a redacted JSON string. Returns empty
 * string for nullish input so callers can chain without guards.
 */
export function redactJson(input: unknown): string {
  if (input == null) return '';
  return JSON.stringify(redactObject(input), null, 2);
}
