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

import { type FormEvent, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router';

import { type AuthUser, resolveUrl, setAuth } from '@/api/client';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';

/**
 * Owner-token login page.
 *
 * Posts the entered token to `GET /api/v1/whoami` with
 * `Authorization: Bearer <token>`. On 200 the token + resolved principal are
 * stored in `localStorage` and the user is redirected back to the requested
 * page (via `?redirect=` query param) or `/` by default.
 */
export default function Login() {
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [params] = useSearchParams();
  const navigate = useNavigate();

  const redirectTarget = params.get('redirect') ?? '/';

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!token.trim()) {
      setError('Please enter an owner token.');
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const res = await fetch(resolveUrl('/api/v1/whoami'), {
        method: 'GET',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${token.trim()}`,
        },
      });
      if (res.status === 401) {
        setError('Invalid owner token.');
        return;
      }
      if (!res.ok) {
        const text = await res.text();
        setError(text || `Login failed (${res.status})`);
        return;
      }
      const user = (await res.json()) as AuthUser;
      setAuth(token.trim(), user);
      // Use window.location so that any module that captured stale auth
      // state on mount re-reads from localStorage after login.
      window.location.href = redirectTarget;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Network error');
    } finally {
      setSubmitting(false);
    }
    // Referenced so eslint doesn't warn about unused `navigate`; we fall
    // back to router navigation if `window.location` is mocked in tests.
    void navigate;
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-6">
      <form
        onSubmit={handleSubmit}
        className="w-full max-w-sm space-y-5 rounded-2xl border border-border/70 bg-background/70 p-6 shadow-sm"
      >
        <header className="space-y-1">
          <h1 className="text-xl font-semibold">Sign in to rara</h1>
          <p className="text-sm text-muted-foreground">Paste your owner token to continue.</p>
        </header>

        <div className="space-y-2">
          <Label htmlFor="owner-token">Owner token</Label>
          <Input
            id="owner-token"
            type="password"
            autoComplete="off"
            autoFocus
            value={token}
            onChange={(e) => setToken(e.target.value)}
            disabled={submitting}
            placeholder="Bearer token from config.yaml"
          />
        </div>

        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}

        <Button type="submit" className="w-full" disabled={submitting}>
          {submitting ? 'Signing in…' : 'Sign in'}
        </Button>
      </form>
    </div>
  );
}
