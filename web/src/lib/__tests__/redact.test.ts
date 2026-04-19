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

import { REDACTED, redactObject } from '../redact';

describe('redactObject', () => {
  it('masks api_key / token / password / authorization / private_key', () => {
    const input = {
      api_key: 'sk-abc',
      apiKey: 'sk-xyz',
      token: 't',
      password: 'p',
      authorization: 'Bearer zz',
      privateKey: 'pk',
      nested: { secret: 's' },
    };
    const out = redactObject(input) as Record<string, unknown>;
    expect(out['api_key']).toBe(REDACTED);
    expect(out['apiKey']).toBe(REDACTED);
    expect(out['token']).toBe(REDACTED);
    expect(out['password']).toBe(REDACTED);
    expect(out['authorization']).toBe(REDACTED);
    expect(out['privateKey']).toBe(REDACTED);
    expect((out['nested'] as Record<string, unknown>)['secret']).toBe(REDACTED);
  });

  it('leaves non-secret keys untouched', () => {
    expect(redactObject({ query: 'hello', count: 3 })).toEqual({ query: 'hello', count: 3 });
  });

  it('handles arrays recursively', () => {
    const out = redactObject([{ token: 't' }, { query: 'q' }]) as unknown[];
    expect((out[0] as Record<string, unknown>)['token']).toBe(REDACTED);
    expect((out[1] as Record<string, unknown>)['query']).toBe('q');
  });
});
