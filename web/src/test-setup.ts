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

// Extends Vitest's `expect` with jest-dom matchers like
// `toBeInTheDocument`. Imported for side effects only.
import '@testing-library/jest-dom/vitest';

// jsdom doesn't implement ResizeObserver, which cmdk (and other
// Radix-adjacent primitives) instantiate on mount. A no-op shim is
// enough for the assertions we run — the components don't exercise
// the observer callback in tests.
if (typeof globalThis.ResizeObserver === 'undefined') {
  class ResizeObserverShim {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  (globalThis as unknown as { ResizeObserver: typeof ResizeObserverShim }).ResizeObserver =
    ResizeObserverShim;
}

// jsdom also lacks `Element.prototype.scrollIntoView`, which cmdk calls
// when auto-scrolling the active item into view. A no-op satisfies the
// interface without affecting assertions.
if (typeof Element !== 'undefined' && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function scrollIntoViewShim(): void {};
}
