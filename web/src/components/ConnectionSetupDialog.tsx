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
import { setBackendUrl, getBackendUrl } from '@/api/client';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

interface ConnectionSetupDialogProps {
  open: boolean;
  onConnect: () => void;
}

/** First-launch dialog that prompts the user to enter their backend URL. */
export function ConnectionSetupDialog({ open, onConnect }: ConnectionSetupDialogProps) {
  const [url, setUrl] = useState(() => getBackendUrl());
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function testConnection() {
    setTesting(true);
    setError(null);
    try {
      const res = await fetch(`${url}/api/v1/settings`, {
        signal: AbortSignal.timeout(5000),
      });
      if (res.ok) {
        setBackendUrl(url);
        onConnect();
      } else {
        setError(`Server returned ${res.status}`);
      }
    } catch (e) {
      setError(`Cannot connect: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setTesting(false);
    }
  }

  return (
    <Dialog open={open}>
      <DialogContent className="sm:max-w-md" onInteractOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>Connect to Rara</DialogTitle>
          <DialogDescription>Enter the URL of your rara backend server.</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <Input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="http://hostname:25555"
            className="font-mono text-sm"
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !testing) testConnection();
            }}
          />
          {error && <p className="text-sm text-destructive">{error}</p>}
          <Button onClick={testConnection} disabled={testing || !url.trim()} className="w-full">
            {testing ? 'Testing...' : 'Connect'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
