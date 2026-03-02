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

import { Outlet } from 'react-router';
import { ShieldAlert } from 'lucide-react';
import { useAuth } from '@/contexts/AuthContext';

export default function AdminRoute() {
  const { isRoot } = useAuth();

  if (!isRoot) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4 text-muted-foreground">
        <ShieldAlert className="h-16 w-16 opacity-30" />
        <h2 className="text-xl font-semibold text-foreground">403 Forbidden</h2>
        <p className="text-sm">You do not have permission to access this page.</p>
      </div>
    );
  }

  return <Outlet />;
}
