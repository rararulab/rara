/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

import { useState, useRef, useCallback } from 'react';
import { buildWsUrl } from '@/adapters/rara-stream';

type VoiceRecorderProps = {
  /** Returns the current session key. */
  getSessionKey: () => string | undefined;
  /** Called when the backend finishes processing the voice message. */
  onComplete?: () => void;
  /** Optional wrapper classes — used to position the floating button. */
  className?: string;
};

/**
 * Convert a Blob to a base64 string (without data-URI prefix).
 */
function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onloadend = () => {
      const result = reader.result as string;
      // Strip "data:...;base64," prefix
      const base64 = result.split(',')[1] ?? '';
      resolve(base64);
    };
    reader.onerror = reject;
    reader.readAsDataURL(blob);
  });
}

/**
 * Floating microphone button for recording voice messages.
 * Records audio via MediaRecorder, sends as an AudioBase64 content block
 * through the existing WebSocket chat API for server-side transcription.
 */
export function VoiceRecorder({ getSessionKey, onComplete, className }: VoiceRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [sending, setSending] = useState(false);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);

  const startRecording = useCallback(async () => {
    const sessionKey = getSessionKey();
    if (!sessionKey) return;

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      let recorder: MediaRecorder;
      try {
        recorder = new MediaRecorder(stream, {
          mimeType: MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
            ? 'audio/webm;codecs=opus'
            : 'audio/webm',
        });
      } catch (err) {
        stream.getTracks().forEach((t) => t.stop());
        console.error('MediaRecorder creation failed:', err);
        return;
      }
      chunksRef.current = [];

      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) chunksRef.current.push(e.data);
      };

      // Capture session key at record start so it stays fixed for the entire
      // record/send lifecycle, even if the user switches sessions mid-recording.
      const capturedSessionKey = sessionKey;

      recorder.onstop = async () => {
        // Stop all tracks to release the microphone.
        stream.getTracks().forEach((t) => t.stop());

        const blob = new Blob(chunksRef.current, { type: recorder.mimeType });
        if (blob.size === 0) return;

        setSending(true);
        try {
          const audioBase64 = await blobToBase64(blob);
          const mimeType = recorder.mimeType.split(';')[0] ?? 'audio/webm';

          // Build JSON payload matching backend InboundPayload with AudioBase64 block.
          const payload = JSON.stringify({
            content: [
              {
                type: 'audio_base64',
                media_type: mimeType,
                data: audioBase64,
              },
            ],
          });

          // Send via WebSocket — same pattern as rara-stream.
          // Keep the connection open until the backend finishes processing,
          // then call onComplete to reload the session messages.
          const wsUrl = buildWsUrl(capturedSessionKey);
          const ws = new WebSocket(wsUrl);

          ws.onopen = () => {
            ws.send(payload);
          };

          ws.onmessage = (ev: MessageEvent) => {
            try {
              const event = JSON.parse(ev.data as string);
              if (event.type === 'done' || event.type === 'error' || event.type === 'message') {
                ws.close();
              }
            } catch {
              // Ignore non-JSON frames
            }
          };

          ws.onerror = () => {
            console.error('Voice WebSocket error');
            setSending(false);
          };

          ws.onclose = () => {
            setSending(false);
            onComplete?.();
          };
        } catch (err) {
          console.error('Voice send error:', err);
          setSending(false);
        }
      };

      recorder.start();
      recorderRef.current = recorder;
      setRecording(true);
    } catch (err) {
      console.error('Microphone access denied:', err);
    }
  }, [getSessionKey, onComplete]);

  const stopRecording = useCallback(() => {
    if (recorderRef.current && recorderRef.current.state === 'recording') {
      recorderRef.current.stop();
      recorderRef.current = null;
      setRecording(false);
    }
  }, []);

  const handleClick = useCallback(() => {
    if (recording) {
      stopRecording();
    } else {
      startRecording();
    }
  }, [recording, startRecording, stopRecording]);

  return (
    <button
      onClick={handleClick}
      disabled={sending}
      className={`flex h-11 w-11 items-center justify-center rounded-full shadow-md transition-all cursor-pointer ${
        recording
          ? 'bg-red-500 text-white animate-pulse hover:bg-red-600'
          : sending
            ? 'bg-muted text-muted-foreground cursor-wait'
            : 'bg-background/80 text-muted-foreground backdrop-blur hover:bg-secondary hover:text-foreground'
      } ${className ?? ''}`}
      title={recording ? 'Stop recording' : sending ? 'Sending...' : 'Record voice message'}
    >
      {sending ? (
        /* Spinner */
        <svg
          className="h-5 w-5 animate-spin"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <circle cx="12" cy="12" r="10" strokeDasharray="60" strokeDashoffset="20" />
        </svg>
      ) : recording ? (
        /* Stop icon */
        <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
          <rect x="6" y="6" width="12" height="12" rx="2" />
        </svg>
      ) : (
        /* Mic icon */
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <rect x="9" y="2" width="6" height="12" rx="3" />
          <path d="M5 10a7 7 0 0 0 14 0" />
          <line x1="12" y1="19" x2="12" y2="22" />
        </svg>
      )}
    </button>
  );
}
