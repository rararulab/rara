/*
 * Copyright 2026 Crrow
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

import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { RuntimeSettingsPatch, RuntimeSettingsView } from "@/api/types";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";

export default function Settings() {
  const queryClient = useQueryClient();
  const [aiModel, setAiModel] = useState("");
  const [aiApiKey, setAiApiKey] = useState("");
  const [telegramToken, setTelegramToken] = useState("");
  const [telegramChatId, setTelegramChatId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.get<RuntimeSettingsView>("/api/v1/settings"),
  });

  useEffect(() => {
    if (!settingsQuery.data) return;
    setAiModel(settingsQuery.data.ai.model ?? "");
    setTelegramChatId(
      settingsQuery.data.telegram.chat_id == null
        ? ""
        : String(settingsQuery.data.telegram.chat_id),
    );
  }, [settingsQuery.data]);

  const patch = useMemo<RuntimeSettingsPatch | null>(() => {
    const current = settingsQuery.data;
    if (!current) return null;
    const next: RuntimeSettingsPatch = {};

    const aiPatch: NonNullable<RuntimeSettingsPatch["ai"]> = {};
    if (aiModel.trim() !== "" && aiModel.trim() !== (current.ai.model ?? "")) {
      aiPatch.model = aiModel.trim();
    }
    if (aiApiKey.trim() !== "") {
      aiPatch.openrouter_api_key = aiApiKey.trim();
    }
    if (Object.keys(aiPatch).length > 0) {
      next.ai = aiPatch;
    }

    const telegramPatch: NonNullable<RuntimeSettingsPatch["telegram"]> = {};
    if (telegramToken.trim() !== "") {
      telegramPatch.bot_token = telegramToken.trim();
    }
    if (telegramChatId.trim() !== "") {
      const parsed = Number.parseInt(telegramChatId.trim(), 10);
      if (!Number.isFinite(parsed)) {
        return null;
      }
      if (parsed !== current.telegram.chat_id) {
        telegramPatch.chat_id = parsed;
      }
    }
    if (Object.keys(telegramPatch).length > 0) {
      next.telegram = telegramPatch;
    }

    return Object.keys(next).length > 0 ? next : null;
  }, [aiApiKey, aiModel, settingsQuery.data, telegramChatId, telegramToken]);

  const updateMutation = useMutation({
    mutationFn: (payload: RuntimeSettingsPatch) =>
      api.post<RuntimeSettingsView>("/api/v1/settings", payload),
    onSuccess: (updated) => {
      queryClient.setQueryData(["settings"], updated);
      setAiApiKey("");
      setTelegramToken("");
      setSuccess("Settings updated successfully.");
      setError(null);
    },
    onError: (e: unknown) => {
      const message = e instanceof Error ? e.message : "Failed to update settings";
      setError(message);
      setSuccess(null);
    },
  });

  const handleSave = () => {
    setError(null);
    setSuccess(null);

    if (!settingsQuery.data) return;
    if (!patch) {
      setError("No valid settings changes to save.");
      return;
    }
    updateMutation.mutate(patch);
  };

  if (settingsQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-56" />
        <Skeleton className="h-72 w-full" />
      </div>
    );
  }

  if (!settingsQuery.data) {
    return (
      <div className="space-y-4">
        <h1 className="text-2xl font-bold">Settings</h1>
        <p className="text-sm text-destructive">
          Failed to load settings. Please refresh and try again.
        </p>
      </div>
    );
  }

  const current = settingsQuery.data;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Settings</h1>
        <p className="text-muted-foreground mt-2">
          Configure runtime credentials without restarting services.
        </p>
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader className="space-y-2">
            <CardTitle className="flex items-center gap-2">
              AI (OpenRouter)
              <Badge variant={current.ai.configured ? "default" : "secondary"}>
                {current.ai.configured ? "Configured" : "Not configured"}
              </Badge>
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="ai-model">Model</Label>
              <Input
                id="ai-model"
                value={aiModel}
                onChange={(e) => setAiModel(e.target.value)}
                placeholder="openai/gpt-4o"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="ai-api-key">OpenRouter API Key</Label>
              <Input
                id="ai-api-key"
                type="password"
                value={aiApiKey}
                onChange={(e) => setAiApiKey(e.target.value)}
                placeholder={current.ai.key_hint ?? "sk-or-v1-..."}
              />
              <p className="text-xs text-muted-foreground">
                Current key hint: {current.ai.key_hint ?? "Not set"}
              </p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="space-y-2">
            <CardTitle className="flex items-center gap-2">
              Telegram Bot
              <Badge variant={current.telegram.configured ? "default" : "secondary"}>
                {current.telegram.configured ? "Configured" : "Not configured"}
              </Badge>
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="telegram-chat-id">Chat ID</Label>
              <Input
                id="telegram-chat-id"
                value={telegramChatId}
                onChange={(e) => setTelegramChatId(e.target.value)}
                placeholder="123456789"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="telegram-token">Bot Token</Label>
              <Input
                id="telegram-token"
                type="password"
                value={telegramToken}
                onChange={(e) => setTelegramToken(e.target.value)}
                placeholder={current.telegram.token_hint ?? "123456:ABC..."}
              />
              <p className="text-xs text-muted-foreground">
                Current token hint: {current.telegram.token_hint ?? "Not set"}
              </p>
            </div>
          </CardContent>
        </Card>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}
      {success && <p className="text-sm text-green-600">{success}</p>}

      <Button onClick={handleSave} disabled={updateMutation.isPending}>
        {updateMutation.isPending ? "Saving..." : "Save Settings"}
      </Button>
    </div>
  );
}
