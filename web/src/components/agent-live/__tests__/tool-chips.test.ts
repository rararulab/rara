/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

import { describe, expect, it } from 'vitest';

import { formatMaybeJson } from '../tool-chips';

describe('formatMaybeJson', () => {
  it('pretty-prints JSON objects with 2-space indent', () => {
    const compact = '{"a":1,"b":[2,3]}';
    expect(formatMaybeJson(compact)).toBe('{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ]\n}');
  });

  it('pretty-prints JSON arrays', () => {
    expect(formatMaybeJson('[1,2,3]')).toBe('[\n  1,\n  2,\n  3\n]');
  });

  it('passes Markdown through unchanged', () => {
    const md = '# Heading\n\n- bullet\n- bullet\n';
    expect(formatMaybeJson(md)).toBe(md);
  });

  it('passes plain text through unchanged', () => {
    expect(formatMaybeJson('hello world')).toBe('hello world');
  });

  it('passes malformed JSON through unchanged', () => {
    const broken = '{"a": 1,';
    expect(formatMaybeJson(broken)).toBe(broken);
  });
});
