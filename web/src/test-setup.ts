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

// jsdom does not implement `DOMMatrix`, which `motion/react` (used by the
// vendor `TurnCard` for expand/collapse animations) reads at module load
// time. A no-op shim is enough — none of our assertions inspect the
// matrix output, and the component falls back to identity geometry.
if (typeof globalThis.DOMMatrix === 'undefined') {
  class DOMMatrixShim {
    a = 1;
    b = 0;
    c = 0;
    d = 1;
    e = 0;
    f = 0;
    m11 = 1;
    m12 = 0;
    m13 = 0;
    m14 = 0;
    m21 = 0;
    m22 = 1;
    m23 = 0;
    m24 = 0;
    m31 = 0;
    m32 = 0;
    m33 = 1;
    m34 = 0;
    m41 = 0;
    m42 = 0;
    m43 = 0;
    m44 = 1;
    is2D = true;
    isIdentity = true;
    multiply(): DOMMatrixShim {
      return new DOMMatrixShim();
    }
    translate(): DOMMatrixShim {
      return new DOMMatrixShim();
    }
    scale(): DOMMatrixShim {
      return new DOMMatrixShim();
    }
  }
  (globalThis as unknown as { DOMMatrix: typeof DOMMatrixShim }).DOMMatrix = DOMMatrixShim;
}
