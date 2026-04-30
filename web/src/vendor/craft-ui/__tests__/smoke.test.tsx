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

/*
 * Vendored craft-agents-oss UI primitives — smoke test.
 *
 * The point of this test is NOT to exercise the components' behavior
 * (craft-agents-oss has its own test suite for that). It's a tripwire
 * for the vendor pipeline itself: if a future refactor breaks the
 * `@/vendor/craft-ui` barrel, drops a peer dependency (motion, clsx,
 * tailwind-merge), or introduces a TS error in one of the vendored
 * files, this test fails at typecheck or at module-load before any
 * real consumer notices.
 *
 * Three components were chosen because they cover the dependency
 * surface: LoadingIndicator → cn only; SimpleDropdown → cn + react-dom
 * portal; Island → cn + dismissible-layer-bridge + motion/react. If
 * all three import cleanly, the vendor barrel is healthy.
 */

import { describe, expect, it } from 'vitest';

import { Island, LoadingIndicator, SimpleDropdown, cn } from '../index';

describe('vendor/craft-ui barrel', () => {
  it('exports LoadingIndicator / SimpleDropdown / Island as functions', () => {
    expect(typeof LoadingIndicator).toBe('function');
    expect(typeof SimpleDropdown).toBe('function');
    expect(typeof Island).toBe('function');
  });

  it('exports the cn helper that merges Tailwind classes', () => {
    expect(cn('px-2', 'px-4')).toBe('px-4');
    expect(cn('text-sm', { 'text-red-500': true, 'text-blue-500': false })).toBe(
      'text-sm text-red-500',
    );
  });
});
