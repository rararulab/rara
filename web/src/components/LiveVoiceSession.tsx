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
// Audio Visualizer — ElevenLabs-inspired waveform bars
// ---------------------------------------------------------------------------

/** Color palette keyed by voice state, adapted for dark theme. */
const STATE_COLORS: Record<VoiceState, string> = {
  idle: "#525252", // zinc-600 — muted, ambient
  sending: "#525252", // same gray, pulsing animation signals "thinking"
  speaking: "#10b981", // emerald-500 — Rara is speaking
};

/** Blue highlight when user is actively speaking (mic input detected). */
const HEARING_COLOR = "#3b82f6"; // blue-500

function AudioVisualizer({
  analyser,
  state,
}: {
  analyser: AnalyserNode | null;
  state: VoiceState;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animFrameRef = useRef<number>(0);
  // Persist per-bar phase offsets for idle breathing animation
  const phaseOffsetsRef = useRef<number[] | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const BAR_COUNT = 40;
    const BAR_WIDTH = 3;
    const BAR_GAP = 2;
    const dataArray = analyser
      ? new Uint8Array(analyser.frequencyBinCount)
      : null;

    // Initialize stable random phase offsets (once)
    if (!phaseOffsetsRef.current) {
      phaseOffsetsRef.current = Array.from(
        { length: BAR_COUNT },
        () => Math.random() * Math.PI * 2,
      );
    }
    const phaseOffsets = phaseOffsetsRef.current;

    // Sending state: soft pulse via opacity oscillation
    let sendingPhase = 0;

    function draw() {
      if (!ctx || !canvas) return;
      const w = canvas.width;
      const h = canvas.height;
      ctx.clearRect(0, 0, w, h);

      if (analyser && dataArray) {
        analyser.getByteFrequencyData(dataArray);
      }

      // Detect whether user is producing audio (hearing state)
      let avgLevel = 0;
      if (dataArray) {
        let sum = 0;
        for (let i = 0; i < dataArray.length; i++) sum += dataArray[i];
        avgLevel = sum / dataArray.length;
      }
      const isHearing = state === "idle" && avgLevel > 12;

      // Pick bar color
      const color = isHearing ? HEARING_COLOR : STATE_COLORS[state];

      // Sending pulse: oscillate global opacity
      let globalAlpha = 1;
      if (state === "sending") {
        sendingPhase += 0.03;
        globalAlpha = 0.4 + 0.3 * Math.sin(sendingPhase);
      }

      const totalBarsWidth = BAR_COUNT * (BAR_WIDTH + BAR_GAP) - BAR_GAP;
      const offsetX = (w - totalBarsWidth) / 2;
      const centerY = h / 2;
      const now = performance.now() / 1000; // seconds

      ctx.lineCap = "round";

      for (let i = 0; i < BAR_COUNT; i++) {
        // Map frequency bin to bar height
        const binIndex = dataArray
          ? Math.floor((i / BAR_COUNT) * dataArray.length)
          : 0;
        const rawValue = dataArray ? dataArray[binIndex] : 0;

        let barHeight: number;
        if (state === "idle" && !isHearing) {
          // Idle breathing: gentle sinusoidal per-bar undulation
          const breath =
            Math.sin(now * 1.2 + phaseOffsets[i]) * 0.5 + 0.5; // 0..1
          barHeight = 3 + breath * 6; // 3..9px — subtle
        } else if (state === "sending") {
          // Thinking: slow wave with moderate height
          const wave =
            Math.sin(now * 2 + (i / BAR_COUNT) * Math.PI * 2) * 0.5 + 0.5;
          barHeight = 4 + wave * 14;
        } else {
          // Hearing or speaking: driven by audio data
          barHeight = Math.max(3, (rawValue / 255) * (h * 0.85));
        }

        const x = offsetX + i * (BAR_WIDTH + BAR_GAP);

        ctx.globalAlpha = globalAlpha;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.roundRect(
          x,
          centerY - barHeight / 2,
          BAR_WIDTH,
          barHeight,
          BAR_WIDTH / 2,
        );
        ctx.fill();
      }

      ctx.globalAlpha = 1;
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
      width={400}
      height={80}
      className="mx-auto block w-[60%] max-w-[400px]"
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
 *
 * UI styled after ElevenLabs design language, adapted for dark theme.
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

  const statusLabel =
    error ??
    (state === "sending"
      ? "THINKING"
      : muted
        ? "MUTED"
        : "LISTENING");

  // ---------------------------------------------------------------------------
  // Render — ElevenLabs-inspired dark voice panel
  // ---------------------------------------------------------------------------

  return (
    <div
      className="absolute inset-x-0 bottom-0 z-50 flex h-52 flex-col items-center justify-center gap-4 border-t border-white/5 bg-background/95 px-4 backdrop-blur-xl"
      style={{
        boxShadow:
          "rgba(255,255,255,0.03) 0px 0px 0px 1px inset, rgba(0,0,0,0.3) 0px -4px 16px",
      }}
    >
      {/* Waveform visualizer — the hero element */}
      <AudioVisualizer analyser={analyser} state={state} />

      {/* Interim transcription or confirmed text */}
      <div className="min-h-[1.5em] w-full max-w-md text-center">
        {state === "sending" && finalText ? (
          <span className="block truncate text-sm tracking-wide text-muted-foreground/70">
            {finalText}
          </span>
        ) : interimText ? (
          <span className="block truncate text-sm italic text-muted-foreground/50">
            {interimText}
          </span>
        ) : null}
      </div>

      {/* Status label — uppercase, tracked, small */}
      <div className="text-[11px] font-medium uppercase tracking-[0.15em] text-muted-foreground/70">
        {statusLabel}
      </div>

      {/* Control bar — pill buttons */}
      <div className="flex items-center gap-8">
        {/* Mute button */}
        <button
          onClick={toggleMute}
          disabled={state === "sending"}
          className={`flex cursor-pointer items-center gap-2 rounded-full px-5 py-2 text-sm transition-all ${
            muted
              ? "bg-red-500/10 text-red-400 hover:bg-red-500/20"
              : "bg-white/5 text-muted-foreground hover:bg-white/10"
          }`}
          title={muted ? "Unmute" : "Mute"}
        >
          {muted ? <MicOff size={16} /> : <Mic size={16} />}
          <span>{muted ? "Unmute" : "Mute"}</span>
        </button>

        {/* End session button */}
        <button
          onClick={handleClose}
          className="flex cursor-pointer items-center gap-2 rounded-full px-5 py-2 text-sm text-red-400 transition-all hover:bg-red-500/10"
          title="End voice session"
        >
          <PhoneOff size={16} />
          <span>End</span>
        </button>
      </div>
    </div>
  );
}
