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

import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { authApi } from "@/api/client";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Check, Clipboard, Loader2, MessageSquare } from "lucide-react";

/** localStorage key — 标记用户已跳过或完成引导 */
const ONBOARDING_DISMISSED_KEY = "onboarding_dismissed";

export function isOnboardingDismissed(): boolean {
  return localStorage.getItem(ONBOARDING_DISMISSED_KEY) === "true";
}

export function dismissOnboarding(): void {
  localStorage.setItem(ONBOARDING_DISMISSED_KEY, "true");
}

interface OnboardingModalProps {
  open: boolean;
  onDismiss: () => void;
}

/**
 * 首次登录引导弹窗 — 引导 root 用户关联 Telegram 账号。
 *
 * 流程：
 * 1. 欢迎说明
 * 2. 生成 link code
 * 3. 复制指令发送给 Telegram Bot
 * 4. 验证关联 / 跳过
 */
export default function OnboardingModal({ open, onDismiss }: OnboardingModalProps) {
  const queryClient = useQueryClient();
  const [linkCode, setLinkCode] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);

  // 生成 link code
  const linkCodeMutation = useMutation({
    mutationFn: () => authApi.generateLinkCode("web_to_tg"),
    onSuccess: (data) => {
      setLinkCode(data.code);
    },
  });

  // 复制指令到剪贴板
  const handleCopy = async (code: string) => {
    await navigator.clipboard.writeText(`/link ${code}`);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  // 验证关联状态
  const handleVerify = async () => {
    setVerifying(true);
    try {
      const profile = await authApi.me();
      // 使 profile query 缓存失效
      queryClient.invalidateQueries({ queryKey: ["profile"] });
      if (profile.platforms && profile.platforms.length > 0) {
        setVerified(true);
        // 标记引导已完成
        dismissOnboarding();
        // 延迟关闭弹窗，让用户看到成功状态
        setTimeout(() => onDismiss(), 1500);
      }
    } finally {
      setVerifying(false);
    }
  };

  // 跳过引导
  const handleSkip = () => {
    dismissOnboarding();
    onDismiss();
  };

  return (
    <Dialog open={open} onOpenChange={(isOpen) => { if (!isOpen) handleSkip(); }}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <MessageSquare className="h-5 w-5 text-primary" />
            欢迎！让我们完成初始设置
          </DialogTitle>
          <DialogDescription>
            关联你的 Telegram 账号，即可通过 Bot 接收通知和进行对话交互。
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          {/* 步骤说明 */}
          {!linkCode && !verified && (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">
                关联流程非常简单：
              </p>
              <ol className="list-inside list-decimal space-y-1.5 text-sm text-muted-foreground">
                <li>点击下方按钮生成一个一次性关联码</li>
                <li>将关联指令发送给 Telegram Bot</li>
                <li>完成！你的账号就关联好了</li>
              </ol>
              <Button
                onClick={() => linkCodeMutation.mutate()}
                disabled={linkCodeMutation.isPending}
                className="w-full"
              >
                {linkCodeMutation.isPending ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    生成中...
                  </>
                ) : (
                  "生成关联码"
                )}
              </Button>
              {linkCodeMutation.isError && (
                <p className="text-sm text-destructive">
                  生成失败，请稍后重试。
                </p>
              )}
            </div>
          )}

          {/* 显示 link code */}
          {linkCode && !verified && (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">
                请将以下指令发送给你的 Telegram Bot：
              </p>
              <div className="flex items-center gap-2 rounded-lg border bg-muted/50 px-3 py-2.5">
                <code className="flex-1 font-mono text-sm font-medium">
                  /link {linkCode}
                </code>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8 shrink-0"
                  onClick={() => handleCopy(linkCode)}
                >
                  {copied ? (
                    <Check className="h-4 w-4 text-green-500" />
                  ) : (
                    <Clipboard className="h-4 w-4" />
                  )}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                发送后，点击下方按钮验证关联是否成功。
              </p>
              <Button
                onClick={handleVerify}
                disabled={verifying}
                variant="outline"
                className="w-full"
              >
                {verifying ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    验证中...
                  </>
                ) : (
                  "验证关联状态"
                )}
              </Button>
            </div>
          )}

          {/* 关联成功 */}
          {verified && (
            <div className="flex flex-col items-center gap-2 py-4">
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/30">
                <Check className="h-6 w-6 text-green-600 dark:text-green-400" />
              </div>
              <p className="text-sm font-medium">Telegram 关联成功！</p>
            </div>
          )}
        </div>

        {!verified && (
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={handleSkip}>
              稍后再说
            </Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}
