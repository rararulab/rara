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

import { useState } from 'react';

import { type AuthUser, getBackendUrl, setAuth, setBackendUrl } from '@/api/client';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';

interface ConnectionSetupDialogProps {
  open: boolean;
  onConnect: () => void;
}

/**
 * First-launch dialog that captures the backend URL and owner token in one
 * step. Probes `/api/v1/whoami` (which lives inside the admin CORS+auth layer)
 * to validate both at once, then persists URL + auth and reloads.
 */
export function ConnectionSetupDialog({ onConnect, open }: ConnectionSetupDialogProps) {
  const [url, setUrl] = useState(() => getBackendUrl());
  const [token, setToken] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleConnect() {
    const trimmedUrl = url.trim().replace(/\/+$/, '');
    const trimmedToken = token.trim();
    if (!trimmedUrl || !trimmedToken) return;

    setSubmitting(true);
    setError(null);
    try {
      const res = await fetch(`${trimmedUrl}/api/v1/whoami`, {
        headers: { Authorization: `Bearer ${trimmedToken}` },
        signal: AbortSignal.timeout(5000),
      });
      if (res.status === 401) {
        setError('Invalid owner token.');
        return;
      }
      if (!res.ok) {
        setError(`Server returned ${res.status}`);
        return;
      }
      const user = (await res.json()) as AuthUser;
      setAuth(trimmedToken, user);
      // setBackendUrl reloads; localStorage already holds auth + URL so the
      // app boots straight into the authenticated route on the next mount.
      setBackendUrl(trimmedUrl);
      onConnect();
    } catch {
      setError('Cannot reach backend at this URL.');
    } finally {
      setSubmitting(false);
    }
  }

  const canSubmit = !!url.trim() && !!token.trim() && !submitting;

  return (
    <Dialog open={open}>
      <DialogContent className="sm:max-w-md" onInteractOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>Connect to Rara</DialogTitle>
          <DialogDescription>
            Enter the URL of your rara backend and your owner token.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="backend-url">Backend URL</Label>
            <Input
              id="backend-url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="http://hostname:25555"
              className="font-mono text-sm"
              disabled={submitting}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="owner-token">Owner token</Label>
            <Input
              id="owner-token"
              type="password"
              autoComplete="off"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="Bearer token from config.yaml"
              disabled={submitting}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && canSubmit) void handleConnect();
              }}
            />
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <Button onClick={handleConnect} disabled={!canSubmit} className="w-full">
            {submitting ? 'Connecting…' : 'Connect'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
