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

/**
 * Craft-style prompt editor pinned to the bottom of the topology
 * timeline. Auto-grows to a max of {@link MAX_TEXTAREA_LINES} lines,
 * sends `prompt` / `abort` frames over the per-session WebSocket, and
 * exposes a toolbar of attachment / mention / voice / model / thinking
 * controls.
 *
 * Wire contract: `crates/channels/src/web_session.rs::InboundFrame`
 * (`{type:"prompt",content}` or `{type:"abort"}`). Multimodal prompts
 * use the `MessageContent::Multimodal` shape — array of
 * `ContentBlock::{Text,ImageBase64,AudioBase64,FileBase64}`. STT runs
 * server-side: the client only base64-encodes the recorded audio and
 * sends it as an `audio_base64` block; the backend pipeline transcribes
 * before dispatch (see `transcribe_audio_blocks`).
 *
 * Per-session model + thinking-level overrides apply session-wide
 * (PATCH `/api/v1/chat/sessions/{key}`) — the backend `Prompt` frame
 * carries no per-turn override slot, and the standing convention is
 * "session = pinned config". Picking a different model changes the
 * pinned config for this session, which takes effect on the next turn.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AtSign, Loader2, Mic, Paperclip, Send, Square, X } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type DragEvent,
  type KeyboardEvent,
} from 'react';

import { SESSIONS_QUERY_KEY } from './SessionPicker';

import type { PromptContent, PromptContentBlock } from '@/agent/session-ws-client';
import { api } from '@/api/client';
import type { ChatSession, ThinkingLevel } from '@/api/types';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { useChatModels, type ChatModelInfo } from '@/hooks/use-chat-models';
import { useChatSessionWs, type ChatSessionWsStatus } from '@/hooks/use-chat-session-ws';
import { useSkills } from '@/hooks/use-skills';
import { cn } from '@/lib/utils';

// ---------------------------------------------------------------------------
// Mechanism constants — not config (see anti-patterns guide)
// ---------------------------------------------------------------------------

/** Soft cap on textarea visible rows. Beyond this, the textarea
 *  scrolls internally instead of pushing the toolbar off-screen. */
const MAX_TEXTAREA_LINES = 8;

/** Hard cap on attachment file size (10 MB). Beyond this the base64
 *  inflation makes the WS frame unwieldy and the kernel's image-handling
 *  paths slow down sharply. */
const MAX_ATTACHMENT_BYTES = 10 * 1024 * 1024;

/** Voice recordings are clamped to keep the base64 payload reasonable.
 *  The backend whisper pipeline accepts longer inputs but the wire
 *  frame would exceed websocket buffer defaults. */
const MAX_VOICE_RECORDING_MS = 60_000;

const THINKING_LEVELS: readonly ThinkingLevel[] = [
  'off',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
] as const;

// ---------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------

interface AttachmentDraft {
  /** Stable id for keyed list rendering. */
  id: string;
  /** File name as the user picked it. */
  name: string;
  /** MIME type the browser reported. Falls back to the kernel-friendly
   *  `application/octet-stream` so the wire payload is always valid. */
  mediaType: string;
  /** Raw base64 (no data: prefix) ready for the wire. */
  data: string;
  /** Bytes — used for the inline preview chip and the max-size guard. */
  size: number;
  /** Whether to render as an image thumbnail (vs a generic file chip). */
  isImage: boolean;
}

/** Strip the `data:<mime>;base64,` prefix that `FileReader.readAsDataURL`
 *  produces. The backend `ContentBlock::ImageBase64` carries the bare
 *  base64 string + a separate `media_type` field. */
function stripDataUrlPrefix(dataUrl: string): string {
  const comma = dataUrl.indexOf(',');
  return comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
}

/** Read a `Blob` (file or recorded audio) into a base64 string + mime. */
async function blobToAttachmentParts(
  blob: Blob,
  fallbackMime: string,
): Promise<{ data: string; mediaType: string }> {
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error ?? new Error('read failed'));
    reader.readAsDataURL(blob);
  });
  return {
    data: stripDataUrlPrefix(dataUrl),
    mediaType: blob.type || fallbackMime,
  };
}

/** Build a `ContentBlock` array for a multimodal prompt. */
function buildMultimodalContent(text: string, attachments: AttachmentDraft[]): PromptContent {
  const blocks: PromptContentBlock[] = [];
  for (const att of attachments) {
    if (att.isImage) {
      blocks.push({
        type: 'image_base64',
        media_type: att.mediaType,
        data: att.data,
      });
    } else if (att.mediaType.startsWith('audio/')) {
      // Voice recording. The backend STT pass
      // (`crates/channels/src/web.rs::transcribe_audio_blocks`) folds
      // every `audio_base64` block into a transcribed `text` block
      // before the kernel sees it.
      blocks.push({
        type: 'audio_base64',
        media_type: att.mediaType,
        data: att.data,
      });
    } else {
      blocks.push({
        type: 'file_base64',
        media_type: att.mediaType,
        data: att.data,
        filename: att.name,
      });
    }
  }
  if (text.trim().length > 0) {
    blocks.push({ type: 'text', text });
  }
  return blocks;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export interface PromptEditorProps {
  /** Session to send into. `null` disables the entire editor. */
  sessionKey: string | null;
}

export function PromptEditor({ sessionKey }: PromptEditorProps) {
  const ws = useChatSessionWs(sessionKey);
  const modelsQuery = useChatModels();
  const skillsQuery = useSkills();
  const queryClient = useQueryClient();

  const sessionQuery = useQuery({
    queryKey: ['topology', 'chat-session', sessionKey] as const,
    queryFn: () =>
      api.get<ChatSession>(`/api/v1/chat/sessions/${encodeURIComponent(sessionKey ?? '')}`),
    enabled: sessionKey !== null,
  });

  const patchSession = useMutation({
    mutationFn: (patch: {
      model?: string | null;
      model_provider?: string | null;
      thinking_level?: ThinkingLevel | null;
    }) =>
      api.patch<ChatSession>(
        `/api/v1/chat/sessions/${encodeURIComponent(sessionKey ?? '')}`,
        patch,
      ),
    onSuccess: (next) => {
      void queryClient.invalidateQueries({ queryKey: SESSIONS_QUERY_KEY });
      queryClient.setQueryData(['topology', 'chat-session', sessionKey], next);
    },
  });

  const [text, setText] = useState('');
  const [attachments, setAttachments] = useState<AttachmentDraft[]>([]);
  const [recording, setRecording] = useState(false);
  const [recordingMs, setRecordingMs] = useState(0);
  const [mentionOpen, setMentionOpen] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const recorderChunksRef = useRef<Blob[]>([]);
  const recordingTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const recordingStopRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const disabled = sessionKey === null || ws.status === 'closed';
  const isStreaming = ws.status === 'streaming';

  // Auto-focus the textarea when the user picks (or switches) a session.
  useEffect(() => {
    if (sessionKey && !disabled) {
      textareaRef.current?.focus();
    }
  }, [sessionKey, disabled]);

  // Auto-grow the textarea — measure scrollHeight, clamp to MAX lines.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    const lineHeight = parseFloat(getComputedStyle(el).lineHeight || '20') || 20;
    const maxHeight = lineHeight * MAX_TEXTAREA_LINES;
    el.style.height = `${Math.min(el.scrollHeight, maxHeight)}px`;
    el.style.overflowY = el.scrollHeight > maxHeight ? 'auto' : 'hidden';
  }, [text]);

  // Tear down recording state if the component unmounts mid-record.
  useEffect(() => {
    return () => {
      if (recordingTimerRef.current) clearInterval(recordingTimerRef.current);
      if (recordingStopRef.current) clearTimeout(recordingStopRef.current);
      const rec = recorderRef.current;
      if (rec && rec.state !== 'inactive') {
        rec.stop();
        rec.stream.getTracks().forEach((t) => t.stop());
      }
    };
  }, []);

  // -------------------------------------------------------------------------
  // Send + abort
  // -------------------------------------------------------------------------

  const handleSend = useCallback(() => {
    if (disabled || isStreaming) return;
    const trimmed = text.trim();
    if (trimmed.length === 0 && attachments.length === 0) return;

    const content: PromptContent =
      attachments.length === 0 ? trimmed : buildMultimodalContent(trimmed, attachments);
    const ok = ws.sendPrompt(content);
    if (ok) {
      setText('');
      setAttachments([]);
      // Re-focus so the user can immediately type the follow-up.
      requestAnimationFrame(() => textareaRef.current?.focus());
    }
  }, [attachments, disabled, isStreaming, text, ws]);

  const handleAbort = useCallback(() => {
    if (!isStreaming) return;
    ws.sendAbort();
  }, [isStreaming, ws]);

  // -------------------------------------------------------------------------
  // Attachments
  // -------------------------------------------------------------------------

  const handleFiles = useCallback(async (files: FileList | File[]) => {
    const accepted: AttachmentDraft[] = [];
    for (const file of Array.from(files)) {
      if (file.size > MAX_ATTACHMENT_BYTES) {
        // Best-effort surfacing — a toast system isn't wired in this
        // crate, so log + skip rather than failing silently.
        console.warn(`attachment too large (${file.name}); max 10 MB`);
        continue;
      }
      const { data, mediaType } = await blobToAttachmentParts(file, 'application/octet-stream');
      accepted.push({
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        name: file.name || 'attachment',
        mediaType,
        data,
        size: file.size,
        isImage: mediaType.startsWith('image/'),
      });
    }
    if (accepted.length > 0) {
      setAttachments((prev) => [...prev, ...accepted]);
    }
  }, []);

  const handleFileInputChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const files = event.target.files;
      if (!files || files.length === 0) return;
      void handleFiles(files);
      // Reset so picking the same file twice still fires `change`.
      event.target.value = '';
    },
    [handleFiles],
  );

  const handleDrop = useCallback(
    (event: DragEvent<HTMLDivElement>) => {
      event.preventDefault();
      if (disabled) return;
      const files = event.dataTransfer.files;
      if (files && files.length > 0) {
        void handleFiles(files);
      }
    },
    [disabled, handleFiles],
  );

  const removeAttachment = useCallback((id: string) => {
    setAttachments((prev) => prev.filter((a) => a.id !== id));
  }, []);

  // -------------------------------------------------------------------------
  // Voice
  // -------------------------------------------------------------------------

  const stopRecording = useCallback(() => {
    const rec = recorderRef.current;
    if (!rec) return;
    if (rec.state !== 'inactive') rec.stop();
  }, []);

  const startRecording = useCallback(async () => {
    if (recording || disabled) return;
    if (typeof navigator === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
      console.warn('voice capture unavailable: no MediaDevices API');
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const rec = new MediaRecorder(stream);
      recorderChunksRef.current = [];
      rec.ondataavailable = (ev) => {
        if (ev.data.size > 0) recorderChunksRef.current.push(ev.data);
      };
      rec.onstop = async () => {
        const chunks = recorderChunksRef.current;
        recorderChunksRef.current = [];
        stream.getTracks().forEach((t) => t.stop());
        if (recordingTimerRef.current) {
          clearInterval(recordingTimerRef.current);
          recordingTimerRef.current = null;
        }
        if (recordingStopRef.current) {
          clearTimeout(recordingStopRef.current);
          recordingStopRef.current = null;
        }
        setRecording(false);
        setRecordingMs(0);
        if (chunks.length === 0) return;
        const mime = rec.mimeType || 'audio/webm';
        const blob = new Blob(chunks, { type: mime });
        if (blob.size > MAX_ATTACHMENT_BYTES) {
          console.warn('recording too large; discarded');
          return;
        }
        const { data, mediaType } = await blobToAttachmentParts(blob, mime);
        setAttachments((prev) => [
          ...prev,
          {
            id: `${Date.now()}-voice`,
            name: `voice-${new Date().toISOString().slice(11, 19)}.webm`,
            mediaType,
            data,
            size: blob.size,
            isImage: false,
          },
        ]);
      };
      recorderRef.current = rec;
      rec.start();
      setRecording(true);
      setRecordingMs(0);
      recordingTimerRef.current = setInterval(() => {
        setRecordingMs((ms) => ms + 250);
      }, 250);
      recordingStopRef.current = setTimeout(() => {
        stopRecording();
      }, MAX_VOICE_RECORDING_MS);
    } catch (err) {
      console.warn('mic permission denied or unavailable', err);
    }
  }, [disabled, recording, stopRecording]);

  // -------------------------------------------------------------------------
  // Mention picker
  // -------------------------------------------------------------------------

  const insertMention = useCallback((skillName: string) => {
    setText((prev) => {
      const needsSpace = prev.length > 0 && !prev.endsWith(' ');
      return `${prev}${needsSpace ? ' ' : ''}@${skillName} `;
    });
    setMentionOpen(false);
    requestAnimationFrame(() => textareaRef.current?.focus());
  }, []);

  // -------------------------------------------------------------------------
  // Keyboard
  // -------------------------------------------------------------------------

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key !== 'Enter') return;
      // Shift+Enter → newline; Enter (or Cmd/Ctrl+Enter) → send.
      if (event.shiftKey) return;
      event.preventDefault();
      handleSend();
    },
    [handleSend],
  );

  // -------------------------------------------------------------------------
  // Render
  // -------------------------------------------------------------------------

  const models = modelsQuery.data ?? [];
  const skills = skillsQuery.data ?? [];
  const session = sessionQuery.data;
  const currentModel = session?.model ?? null;
  const currentThinking = session?.thinking_level ?? null;

  return (
    <div
      className={cn(
        'sticky bottom-0 z-10 mt-2 rounded-lg border border-border bg-card shadow-sm',
        'flex flex-col gap-2 p-3',
        disabled && 'opacity-60',
      )}
      onDragOver={(e) => {
        if (!disabled) e.preventDefault();
      }}
      onDrop={handleDrop}
    >
      {ws.error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1 text-xs text-destructive">
          {ws.error}
        </div>
      )}

      <textarea
        ref={textareaRef}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={handleKeyDown}
        disabled={disabled}
        placeholder={
          sessionKey === null
            ? 'Select a session to start'
            : ws.status === 'reconnecting'
              ? 'Reconnecting…'
              : 'Message rara'
        }
        rows={1}
        className={cn(
          'w-full resize-none border-0 bg-transparent px-1 py-1 text-sm leading-5',
          'focus:outline-none focus-visible:outline-none',
          'placeholder:text-muted-foreground',
        )}
      />

      {attachments.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {attachments.map((att) => (
            <AttachmentChip key={att.id} attachment={att} onRemove={removeAttachment} />
          ))}
        </div>
      )}

      <div className="flex items-center gap-1">
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={handleFileInputChange}
        />
        <ToolbarIconButton
          title="Attach file"
          disabled={disabled}
          onClick={() => fileInputRef.current?.click()}
          icon={<Paperclip className="h-3.5 w-3.5" />}
        />

        <DropdownMenu open={mentionOpen} onOpenChange={setMentionOpen}>
          <DropdownMenuTrigger asChild>
            <span>
              <ToolbarIconButton
                title="Mention skill"
                disabled={disabled || skills.length === 0}
                icon={<AtSign className="h-3.5 w-3.5" />}
              />
            </span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="max-h-72 w-64 overflow-y-auto">
            {skillsQuery.isLoading && (
              <div className="px-2 py-1.5 text-xs text-muted-foreground">Loading skills…</div>
            )}
            {!skillsQuery.isLoading && skills.length === 0 && (
              <div className="px-2 py-1.5 text-xs text-muted-foreground">No skills available</div>
            )}
            {skills.map((skill) => (
              <DropdownMenuItem
                key={skill.name}
                onSelect={(e) => {
                  e.preventDefault();
                  insertMention(skill.name);
                }}
                className={cn('flex flex-col items-start', !skill.eligible && 'opacity-50')}
              >
                <span className="font-mono text-xs">@{skill.name}</span>
                {skill.description && (
                  <span className="line-clamp-2 text-[11px] text-muted-foreground">
                    {skill.description}
                  </span>
                )}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>

        <div className="ml-auto flex items-center gap-1">
          <ToolbarIconButton
            title={recording ? 'Stop recording' : 'Record voice'}
            disabled={disabled}
            onClick={() => (recording ? stopRecording() : void startRecording())}
            icon={<Mic className={cn('h-3.5 w-3.5', recording && 'text-destructive')} />}
            {...(recording ? { label: formatRecordingTime(recordingMs) } : {})}
            active={recording}
          />

          <ModelPickerButton
            current={currentModel}
            models={models}
            disabled={disabled || sessionKey === null}
            onPick={(model) =>
              patchSession.mutate({
                model,
                // Clear the provider override when the user picks a new
                // model — keeping a stale provider would re-route to a
                // different driver and break the selection.
                model_provider: null,
              })
            }
          />

          <ThinkingPickerButton
            current={currentThinking}
            disabled={disabled || sessionKey === null}
            onPick={(level) => patchSession.mutate({ thinking_level: level })}
          />

          {isStreaming ? (
            <Button
              size="sm"
              variant="destructive"
              className="h-7 gap-1 px-2"
              onClick={handleAbort}
              title="Stop"
            >
              <Square className="h-3 w-3" />
              <span className="text-[11px]">Stop</span>
            </Button>
          ) : (
            <Button
              size="sm"
              className="h-7 gap-1 px-2"
              onClick={handleSend}
              disabled={
                disabled ||
                ws.status === 'connecting' ||
                ws.status === 'reconnecting' ||
                (text.trim().length === 0 && attachments.length === 0)
              }
              title="Send (Enter)"
            >
              {ws.status === 'connecting' || ws.status === 'reconnecting' ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <Send className="h-3 w-3" />
              )}
              <span className="text-[11px]">Send</span>
            </Button>
          )}
        </div>
      </div>

      <div className="flex items-center justify-between text-[10px] text-muted-foreground">
        <span>
          <StatusLabel status={ws.status} />
        </span>
        <span>
          <kbd className="rounded border border-border bg-muted px-1 py-0.5 font-mono text-[10px]">
            Enter
          </kbd>{' '}
          send ·{' '}
          <kbd className="rounded border border-border bg-muted px-1 py-0.5 font-mono text-[10px]">
            Shift+Enter
          </kbd>{' '}
          newline
        </span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Subcomponents
// ---------------------------------------------------------------------------

function ToolbarIconButton({
  icon,
  title,
  onClick,
  disabled,
  label,
  active,
}: {
  icon: React.ReactNode;
  title: string;
  onClick?: () => void;
  disabled?: boolean;
  label?: string;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        'flex h-7 items-center gap-1 rounded-md px-1.5 text-muted-foreground transition-colors',
        'hover:bg-accent hover:text-foreground',
        'disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent',
        active && 'bg-destructive/10 text-destructive hover:text-destructive',
      )}
    >
      {icon}
      {label && <span className="text-[11px] tabular-nums">{label}</span>}
    </button>
  );
}

function AttachmentChip({
  attachment,
  onRemove,
}: {
  attachment: AttachmentDraft;
  onRemove: (id: string) => void;
}) {
  return (
    <div className="flex items-center gap-1.5 rounded-md border border-border bg-muted/40 px-1.5 py-1 text-xs">
      {attachment.isImage ? (
        // Inline preview for images — keeps the chip compact while still
        // letting the user verify what they attached before sending.
        <img
          src={`data:${attachment.mediaType};base64,${attachment.data}`}
          alt={attachment.name}
          className="h-8 w-8 rounded object-cover"
        />
      ) : (
        <Paperclip className="h-3.5 w-3.5 text-muted-foreground" />
      )}
      <span className="max-w-[140px] truncate">{attachment.name}</span>
      <span className="text-muted-foreground">{formatBytes(attachment.size)}</span>
      <button
        type="button"
        title="Remove attachment"
        className="flex h-4 w-4 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
        onClick={() => onRemove(attachment.id)}
      >
        <X className="h-3 w-3" />
      </button>
    </div>
  );
}

function ModelPickerButton({
  current,
  models,
  disabled,
  onPick,
}: {
  current: string | null;
  models: ChatModelInfo[];
  disabled: boolean;
  onPick: (modelId: string | null) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          title="Model"
          className={cn(
            'flex h-7 max-w-[160px] items-center gap-1 truncate rounded-md px-2 text-[11px] text-muted-foreground transition-colors',
            'hover:bg-accent hover:text-foreground',
            'disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent',
          )}
        >
          <span className="truncate">{current ?? 'auto'}</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-72 w-64 overflow-y-auto">
        <DropdownMenuItem
          className="flex flex-col items-start"
          onSelect={(e) => {
            e.preventDefault();
            onPick(null);
          }}
        >
          <span className="text-xs">auto</span>
          <span className="text-[11px] text-muted-foreground">use admin default</span>
        </DropdownMenuItem>
        {models.map((m) => (
          <DropdownMenuItem
            key={m.id}
            className="flex flex-col items-start"
            onSelect={(e) => {
              e.preventDefault();
              onPick(m.id);
            }}
          >
            <span className="text-xs">{m.name}</span>
            <span className="font-mono text-[10px] text-muted-foreground">{m.id}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ThinkingPickerButton({
  current,
  disabled,
  onPick,
}: {
  current: ThinkingLevel | null;
  disabled: boolean;
  onPick: (level: ThinkingLevel | null) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          title="Thinking level"
          className={cn(
            'flex h-7 items-center gap-1 rounded-md px-2 text-[11px] text-muted-foreground transition-colors',
            'hover:bg-accent hover:text-foreground',
            'disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent',
          )}
        >
          <span>think: {current ?? 'auto'}</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuItem
          onSelect={(e) => {
            e.preventDefault();
            onPick(null);
          }}
        >
          <span className="text-xs">auto</span>
        </DropdownMenuItem>
        {THINKING_LEVELS.map((level) => (
          <DropdownMenuItem
            key={level}
            onSelect={(e) => {
              e.preventDefault();
              onPick(level);
            }}
          >
            <span className="text-xs">{level}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function StatusLabel({ status }: { status: ChatSessionWsStatus }) {
  switch (status) {
    case 'idle':
      return <>no session</>;
    case 'connecting':
      return <>connecting…</>;
    case 'live':
      return <>ready</>;
    case 'streaming':
      return <>streaming…</>;
    case 'reconnecting':
      return <>reconnecting…</>;
    case 'closed':
      return <>disconnected</>;
  }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatRecordingTime(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

// Re-exported for tests that want to inspect the constants without
// importing them by file path.
export const __test = {
  MAX_TEXTAREA_LINES,
  MAX_ATTACHMENT_BYTES,
  MAX_VOICE_RECORDING_MS,
  buildMultimodalContent,
  stripDataUrlPrefix,
};
