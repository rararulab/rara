/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

import { useState, useEffect, useRef, useCallback } from "react";
import { Mic, MicOff, PhoneOff } from "lucide-react";
import { buildWsUrl } from "@/adapters/rara-stream";

// ---------------------------------------------------------------------------
// Web Speech API type declarations
// The Web Speech API is not fully standardized and TypeScript's lib.dom does
// not include SpeechRecognition / SpeechRecognitionEvent. We declare the
// subset we use here to avoid pulling in @types/dom-speech-recognition.
// ---------------------------------------------------------------------------

interface SpeechRecognitionEvent extends Event {
  readonly resultIndex: number;
  readonly results: SpeechRecognitionResultList;
}

interface SpeechRecognitionErrorEvent extends Event {
  readonly error: string;
  readonly message: string;
}

interface SpeechRecognitionInstance extends EventTarget {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  start(): void;
  stop(): void;
  abort(): void;
  onresult: ((event: SpeechRecognitionEvent) => void) | null;
  onerror: ((event: SpeechRecognitionErrorEvent) => void) | null;
  onend: (() => void) | null;
}

interface SpeechRecognitionConstructor {
  new (): SpeechRecognitionInstance;
}

declare global {
  interface Window {
    SpeechRecognition?: SpeechRecognitionConstructor;
    webkitSpeechRecognition?: SpeechRecognitionConstructor;
  }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type VoiceState = "idle" | "sending" | "speaking";

type LiveVoiceSessionProps = {
  /** Returns the current session key for WebSocket connections. */
  getSessionKey: () => string | undefined;
  /** Called when the backend finishes processing one turn. */
  onTurnComplete: () => void;
  /** Called when the user ends the live voice session. */
  onClose: () => void;
};

// ---------------------------------------------------------------------------
// Audio Visualizer (inline — replaces LiveKit Agents UI dependency)
// ---------------------------------------------------------------------------

function AudioVisualizer({
  analyser,
  state,
}: {
  analyser: AnalyserNode | null;
  state: VoiceState;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animFrameRef = useRef<number>(0);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const BAR_COUNT = 32;
    const dataArray = analyser ? new Uint8Array(analyser.frequencyBinCount) : null;

    function draw() {
      if (!ctx || !canvas) return;
      const w = canvas.width;
      const h = canvas.height;
      ctx.clearRect(0, 0, w, h);

      // Determine color based on state
      const color =
        state === "sending"
          ? "rgba(156, 163, 175, 0.5)" // gray — thinking
          : state === "speaking"
            ? "rgba(34, 197, 94, 0.7)" // green — speaking
            : "rgba(59, 130, 246, 0.6)"; // blue — listening

      if (analyser && dataArray) {
        analyser.getByteFrequencyData(dataArray);
      }

      const barWidth = w / BAR_COUNT - 2;
      const centerY = h / 2;

      for (let i = 0; i < BAR_COUNT; i++) {
        // Map frequency bin to bar height
        const binIndex = dataArray
          ? Math.floor((i / BAR_COUNT) * dataArray.length)
          : 0;
        const value = dataArray ? dataArray[binIndex] : 0;
        // Minimum bar height for idle state
        const barHeight = Math.max(2, (value / 255) * (h * 0.8));

        const x = i * (barWidth + 2) + 1;
        ctx.fillStyle = color;
        ctx.roundRect(x, centerY - barHeight / 2, barWidth, barHeight, 2);
        ctx.fill();
      }

      animFrameRef.current = requestAnimationFrame(draw);
    }

    draw();

    return () => {
      cancelAnimationFrame(animFrameRef.current);
    };
  }, [analyser, state]);

  return (
    <canvas
      ref={canvasRef}
      width={320}
      height={48}
      className="mx-auto block"
    />
  );
}

// ---------------------------------------------------------------------------
// LiveVoiceSession — main component
// ---------------------------------------------------------------------------

/**
 * Bottom voice panel for real-time voice conversation.
 * Uses Web Speech API for continuous speech-to-text, sends transcribed text
 * through the existing WebSocket chat API, and displays a waveform visualizer.
 */
export function LiveVoiceSession({
  getSessionKey,
  onTurnComplete,
  onClose,
}: LiveVoiceSessionProps) {
  const [state, setState] = useState<VoiceState>("idle");
  const [muted, setMuted] = useState(false);
  const [interimText, setInterimText] = useState("");
  const [finalText, setFinalText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [analyser, setAnalyser] = useState<AnalyserNode | null>(null);

  // Refs for cleanup-safe access
  const recognitionRef = useRef<SpeechRecognitionInstance | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const liveModeRef = useRef(true);
  const mutedRef = useRef(false);

  // Keep mutedRef in sync with muted state
  useEffect(() => {
    mutedRef.current = muted;
  }, [muted]);

  // ---------------------------------------------------------------------------
  // Speech recognition management
  // ---------------------------------------------------------------------------

  const resumeRecognition = useCallback(() => {
    if (!liveModeRef.current || mutedRef.current) return;
    try {
      recognitionRef.current?.start();
    } catch {
      // May already be running
    }
  }, []);

  // ---------------------------------------------------------------------------
  // Send transcribed text to backend via WebSocket
  // ---------------------------------------------------------------------------

  const sendText = useCallback(
    (text: string) => {
      const sessionKey = getSessionKey();
      if (!sessionKey || !text.trim()) return;

      setState("sending");
      setFinalText(text);
      setInterimText("");

      // Pause recognition while waiting for response
      try {
        recognitionRef.current?.stop();
      } catch {
        // May already be stopped
      }

      const wsUrl = buildWsUrl(sessionKey);
      const ws = new WebSocket(wsUrl);
      wsRef.current = ws;

      ws.onopen = () => {
        ws.send(text);
      };

      ws.onmessage = (ev: MessageEvent) => {
        try {
          const event = JSON.parse(ev.data as string);
          if (event.type === "done" || event.type === "message") {
            ws.close();
          } else if (event.type === "error") {
            console.error("Voice WS error:", event.message);
            ws.close();
          }
        } catch {
          // Ignore non-JSON frames
        }
      };

      ws.onerror = () => {
        console.error("Voice WebSocket connection error");
        setState("idle");
        resumeRecognition();
      };

      ws.onclose = () => {
        wsRef.current = null;
        onTurnComplete();
        setState("idle");
        setFinalText("");
        resumeRecognition();
      };
    },
    [getSessionKey, onTurnComplete, resumeRecognition],
  );

  // Initialize speech recognition and microphone audio
  useEffect(() => {
    const SpeechRecognitionCtor =
      window.SpeechRecognition ?? window.webkitSpeechRecognition;
    if (!SpeechRecognitionCtor) {
      setError("Speech recognition is not supported in this browser.");
      return;
    }

    const recognition = new SpeechRecognitionCtor();
    recognition.continuous = true;
    recognition.interimResults = true;
    recognition.lang = "zh-CN";
    recognitionRef.current = recognition;

    recognition.onresult = (event) => {
      const result = event.results[event.resultIndex];
      if (result.isFinal) {
        const transcript = result[0].transcript.trim();
        if (transcript) {
          sendText(transcript);
        }
      } else {
        setInterimText(result[0].transcript);
      }
    };

    recognition.onerror = (event) => {
      // "no-speech" and "aborted" are expected during normal operation
      if (event.error === "no-speech" || event.error === "aborted") return;
      console.error("Speech recognition error:", event.error);
      if (event.error === "not-allowed") {
        setError("Microphone access denied. Please allow microphone access.");
      }
    };

    recognition.onend = () => {
      // Auto-restart if still in live mode and not muted
      if (liveModeRef.current && !mutedRef.current) {
        try {
          recognition.start();
        } catch {
          // May fail if already started
        }
      }
    };

    // Start listening
    try {
      recognition.start();
    } catch (err) {
      console.error("Failed to start speech recognition:", err);
      setError("Failed to start speech recognition.");
    }

    // Set up AudioContext for visualizer
    navigator.mediaDevices
      .getUserMedia({ audio: true })
      .then((stream) => {
        streamRef.current = stream;
        const audioCtx = new AudioContext();
        audioCtxRef.current = audioCtx;
        const source = audioCtx.createMediaStreamSource(stream);
        const analyserNode = audioCtx.createAnalyser();
        analyserNode.fftSize = 256;
        source.connect(analyserNode);
        // Do NOT connect to destination — we don't want to hear our own mic
        setAnalyser(analyserNode);
      })
      .catch((err) => {
        console.error("Microphone access for visualizer failed:", err);
        // Non-fatal — visualizer just won't work
      });

    // Cleanup on unmount
    return () => {
      liveModeRef.current = false;
      try {
        recognition.stop();
      } catch {
        // ignore
      }
      recognitionRef.current = null;
      wsRef.current?.close();
      wsRef.current = null;
      streamRef.current?.getTracks().forEach((t) => t.stop());
      audioCtxRef.current?.close();
    };
  }, [sendText]);

  // ---------------------------------------------------------------------------
  // Mute / unmute
  // ---------------------------------------------------------------------------

  const toggleMute = useCallback(() => {
    setMuted((prev) => {
      const next = !prev;
      if (next) {
        // Muting — stop recognition
        try {
          recognitionRef.current?.stop();
        } catch {
          // ignore
        }
      } else {
        // Unmuting — restart recognition
        try {
          recognitionRef.current?.start();
        } catch {
          // ignore
        }
      }
      return next;
    });
  }, []);

  // ---------------------------------------------------------------------------
  // Close session
  // ---------------------------------------------------------------------------

  const handleClose = useCallback(() => {
    liveModeRef.current = false;
    try {
      recognitionRef.current?.stop();
    } catch {
      // ignore
    }
    wsRef.current?.close();
    onClose();
  }, [onClose]);

  // ---------------------------------------------------------------------------
  // Status text
  // ---------------------------------------------------------------------------

  const statusText =
    error ??
    (state === "sending"
      ? "Thinking..."
      : muted
        ? "Muted"
        : "Listening...");

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div className="absolute inset-x-0 bottom-0 z-50 flex h-48 flex-col items-center justify-center gap-3 border-t bg-background/95 px-4 backdrop-blur">
      {/* Waveform visualizer */}
      <AudioVisualizer analyser={analyser} state={state} />

      {/* Interim transcription or confirmed text */}
      <div className="h-5 w-full max-w-md text-center">
        {state === "sending" && finalText ? (
          <span className="text-sm text-foreground truncate block">
            {finalText}
          </span>
        ) : interimText ? (
          <span className="text-sm italic text-muted-foreground truncate block">
            {interimText}
          </span>
        ) : null}
      </div>

      {/* Status text */}
      <div className="text-xs text-muted-foreground">{statusText}</div>

      {/* Control bar */}
      <div className="flex items-center gap-4">
        {/* Mute button */}
        <button
          onClick={toggleMute}
          disabled={state === "sending"}
          className={`flex h-10 w-10 cursor-pointer items-center justify-center rounded-full transition-colors ${
            muted
              ? "bg-destructive text-destructive-foreground hover:bg-destructive/90"
              : "bg-secondary text-secondary-foreground hover:bg-secondary/80"
          }`}
          title={muted ? "Unmute" : "Mute"}
        >
          {muted ? <MicOff size={18} /> : <Mic size={18} />}
        </button>

        {/* End session button */}
        <button
          onClick={handleClose}
          className="flex h-10 w-10 cursor-pointer items-center justify-center rounded-full bg-destructive text-destructive-foreground transition-colors hover:bg-destructive/90"
          title="End voice session"
        >
          <PhoneOff size={18} />
        </button>
      </div>
    </div>
  );
}
