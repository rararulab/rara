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

import { describe, expect, it } from 'vitest';

import { toolSummary } from '../tool-summary';

describe('toolSummary', () => {
  it('picks query when present without description', () => {
    expect(toolSummary({ query: 'rust trait bounds' })).toBe('rust trait bounds');
  });

  it('prefers description over every other field when present', () => {
    const summary = toolSummary({
      description: 'Compile and run tests',
      query: 'cargo test',
      command: 'cargo test',
      file_path: '/a/b/c.rs',
    });
    expect(summary).toBe('Compile and run tests');
  });

  it('shortens deep file paths', () => {
    expect(toolSummary({ file_path: '/Users/ryan/code/rara/web/src/main.tsx' })).toBe(
      '.../src/main.tsx',
    );
  });

  it('caps command at 100 chars with ellipsis', () => {
    const cmd = 'echo ' + 'x'.repeat(200);
    const out = toolSummary({ command: cmd });
    expect(out.endsWith('...')).toBe(true);
    // 100 chars + 3 ellipsis
    expect(out.length).toBe(103);
  });

  it('returns empty string for empty input', () => {
    expect(toolSummary({})).toBe('');
    expect(toolSummary(null)).toBe('');
    expect(toolSummary(undefined)).toBe('');
  });
});
