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

import { useQuery } from '@tanstack/react-query';
import { AlertCircle, ShieldCheck } from 'lucide-react';

import { api } from '@/api/client';
import { Badge } from '@/components/ui/badge';
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet';
import { Skeleton } from '@/components/ui/skeleton';

interface ApprovalRequest {
  id: string;
  session_key: string;
  tool_name: string;
  description: string;
  created_at: string;
}

export interface ApprovalsDrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/**
 * Side drawer listing pending guard approval requests.
 *
 * Data source: `GET /api/v1/kernel/approvals`.
 */
export function ApprovalsDrawer({ open, onOpenChange }: ApprovalsDrawerProps) {
  const query = useQuery({
    queryKey: ['kernel-approvals'],
    queryFn: () => api.get<ApprovalRequest[]>('/api/v1/kernel/approvals'),
    enabled: open,
    refetchInterval: open ? 5_000 : false,
  });

  const approvals = query.data ?? [];

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-[380px] sm:w-[440px]">
        <SheetHeader>
          <SheetTitle className="flex items-center gap-2">
            <ShieldCheck className="h-4 w-4" />
            Pending Approvals
            {approvals.length > 0 && (
              <Badge variant="destructive" className="text-[10px]">
                {approvals.length}
              </Badge>
            )}
          </SheetTitle>
          <SheetDescription>Guard approval requests awaiting your decision.</SheetDescription>
        </SheetHeader>

        <div className="mt-4 space-y-2">
          {query.isLoading ? (
            <>
              <Skeleton className="h-16 w-full" />
              <Skeleton className="h-16 w-full" />
            </>
          ) : approvals.length === 0 ? (
            <div className="flex flex-col items-center gap-2 py-12 text-center text-muted-foreground">
              <ShieldCheck className="h-8 w-8 opacity-20" />
              <p className="text-sm">No pending approvals</p>
            </div>
          ) : (
            approvals.map((req) => (
              <div key={req.id} className="rounded-lg border p-3 text-xs space-y-1">
                <div className="flex items-center gap-2">
                  <AlertCircle className="h-3.5 w-3.5 shrink-0 text-warning" />
                  <span className="font-medium text-foreground">{req.tool_name}</span>
                  <span className="ml-auto font-mono text-[10px] text-muted-foreground/50">
                    {req.session_key.slice(0, 8)}
                  </span>
                </div>
                <p className="text-muted-foreground">{req.description}</p>
                <p className="text-[10px] text-muted-foreground/50">
                  {new Date(req.created_at).toLocaleString(undefined, {
                    month: 'short',
                    day: 'numeric',
                    hour: '2-digit',
                    minute: '2-digit',
                  })}
                </p>
              </div>
            ))
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
