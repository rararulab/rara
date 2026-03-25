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

import { useState, useCallback, type FormEvent } from 'react';
import { useNavigate } from 'react-router';
import { useAuth } from '@/contexts/AuthContext';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';

export default function Login() {
  const { authenticate } = useAuth();
  const navigate = useNavigate();
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const handleSubmit = useCallback(
    async (e: FormEvent) => {
      e.preventDefault();
      setError(null);

      const trimmed = token.trim();
      if (!trimmed) {
        setError('Please enter your access token.');
        return;
      }

      setIsSubmitting(true);
      try {
        authenticate(trimmed);
        navigate('/agent', { replace: true });
      } catch (err) {
        const message = err instanceof Error ? err.message : 'Authentication failed';
        setError(message);
      } finally {
        setIsSubmitting(false);
      }
    },
    [token, authenticate, navigate],
  );

  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-4">
      <div className="w-full max-w-sm animate-enter">
        {/* Brand */}
        <div className="mb-8 text-center">
          <h1 className="text-display text-foreground">rara</h1>
          <p className="mt-2 text-body text-muted-foreground">
            Your personal AI agent
          </p>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="space-y-4">
          {error && (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              {error}
            </div>
          )}
          <div className="space-y-2">
            <Label htmlFor="token" className="text-caption uppercase tracking-wider">
              Access Token
            </Label>
            <Input
              id="token"
              type="password"
              placeholder="your owner token"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              className="h-11 rounded-xl border-border/60 bg-card text-sm"
              autoFocus
            />
          </div>
          <Button type="submit" className="h-11 w-full rounded-xl text-sm font-semibold" disabled={isSubmitting}>
            {isSubmitting ? 'Signing in...' : 'Sign In'}
          </Button>
        </form>
      </div>
    </div>
  );
}
