/*
 * Copyright 2025 Crrow
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
import { Outlet, useLocation, useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { LogOut, ShieldCheck, User } from 'lucide-react';
import { cn } from '@/lib/utils';
import { useAuth } from '@/contexts/AuthContext';
import { authApi } from '@/api/client';
import { Button } from '@/components/ui/button';
import OnboardingModal, { isOnboardingDismissed } from '@/components/OnboardingModal';

/** Routes that need zero padding in the main content area. */
const FULL_BLEED_ROUTES = new Set(['/agent', '/jobs', '/docs']);

/** Routes that need full bleed when they match as a prefix. */
const FULL_BLEED_PREFIXES: string[] = [];

export default function DashboardLayout() {
  const location = useLocation();
  const navigate = useNavigate();
  const { user, isRoot, logout } = useAuth();
  const isFullBleed = FULL_BLEED_ROUTES.has(location.pathname) || FULL_BLEED_PREFIXES.some(p => location.pathname.startsWith(p));

  // 获取用户 profile（含 platforms）以判断是否需要引导
  const profileQuery = useQuery({
    queryKey: ['profile'],
    queryFn: () => authApi.me(),
    enabled: isRoot,
  });

  // 判断是否显示引导弹窗：root 用户 + 无已关联平台 + 未跳过引导
  const shouldShowOnboarding =
    isRoot &&
    profileQuery.isSuccess &&
    profileQuery.data.platforms.length === 0 &&
    !isOnboardingDismissed();

  const [onboardingOpen, setOnboardingOpen] = useState(true);

  const handleOnboardingDismiss = () => {
    setOnboardingOpen(false);
  };

  const handleLogout = () => {
    logout();
    navigate('/login', { replace: true });
  };

  return (
    <div className="flex h-screen bg-transparent">
      {/* 首次登录引导弹窗 */}
      {shouldShowOnboarding && (
        <OnboardingModal
          open={onboardingOpen}
          onDismiss={handleOnboardingDismiss}
        />
      )}

      <main className={cn('relative flex min-w-0 flex-1 flex-col', isFullBleed ? 'overflow-hidden' : 'overflow-auto')}>
        {/* Top bar with user info */}
        <div className="flex shrink-0 items-center justify-end gap-2 border-b border-border/40 bg-background/30 px-4 py-1.5 backdrop-blur-sm">
          {isRoot && (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
              onClick={() => navigate('/admin/users')}
            >
              <ShieldCheck className="h-3.5 w-3.5" />
              Admin
            </Button>
          )}
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <User className="h-3.5 w-3.5" />
            <span>{user?.name ?? 'Unknown'}</span>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-destructive"
            onClick={handleLogout}
            title="Sign out"
          >
            <LogOut className="h-3.5 w-3.5" />
            Logout
          </Button>
        </div>

        <div className={cn('flex-1 min-h-0', isFullBleed ? 'p-2 md:p-3' : 'p-4 md:p-6')}>
          <Outlet />
        </div>
      </main>
    </div>
  );
}
