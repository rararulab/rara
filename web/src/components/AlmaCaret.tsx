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

import { useEffect, useRef, useState } from "react";

interface AlmaCaretProps {
  /**
   * Bump this when an ancestor of the textarea animates position so
   * the caret is re-measured both immediately and after the 420 ms
   * layout transition finishes. Bumping the key is how PiChat tells
   * the caret that welcome-mode was toggled.
   */
  measureKey?: string | number;
}

/**
 * Mirrored textarea trick: copy every layout-affecting computed style onto a
 * hidden `<div>`, drop in the text up to the caret followed by a marker
 * `<span>`, and read back the marker's bounding rect. That rect is where
 * the real caret would sit — the textarea itself exposes no such API.
 */
const MIRROR_STYLE_KEYS = [
  "boxSizing",
  "width",
  "height",
  "paddingTop",
  "paddingRight",
  "paddingBottom",
  "paddingLeft",
  "borderTopWidth",
  "borderRightWidth",
  "borderBottomWidth",
  "borderLeftWidth",
  "fontFamily",
  "fontSize",
  "fontWeight",
  "fontStyle",
  "lineHeight",
  "letterSpacing",
  "wordSpacing",
  "textTransform",
  "textIndent",
  "tabSize",
] as const;

interface CaretPos {
  x: number;
  y: number;
  height: number;
}

function measureCaret(textarea: HTMLTextAreaElement): CaretPos | null {
  if (!textarea.isConnected) return null;

  const cs = getComputedStyle(textarea);
  const mirror = document.createElement("div");
  for (const key of MIRROR_STYLE_KEYS) {
    mirror.style[key as never] = cs[key as never];
  }
  mirror.style.position = "absolute";
  mirror.style.visibility = "hidden";
  mirror.style.top = "0";
  mirror.style.left = "0";
  mirror.style.whiteSpace = "pre-wrap";
  mirror.style.wordWrap = "break-word";
  mirror.style.overflow = "hidden";

  const end = textarea.selectionEnd ?? textarea.value.length;
  mirror.textContent = textarea.value.substring(0, end);
  const marker = document.createElement("span");
  // U+200B (zero-width space) keeps the line alive without adding glyph width.
  marker.textContent = "\u200b";
  mirror.appendChild(marker);

  document.body.appendChild(mirror);
  const markerRect = marker.getBoundingClientRect();
  const mirrorRect = mirror.getBoundingClientRect();
  document.body.removeChild(mirror);

  const taRect = textarea.getBoundingClientRect();
  const lineHeight =
    parseFloat(cs.lineHeight) ||
    parseFloat(cs.fontSize) * 1.3 ||
    18;

  return {
    x: taRect.left + (markerRect.left - mirrorRect.left) - textarea.scrollLeft,
    y: taRect.top + (markerRect.top - mirrorRect.top) - textarea.scrollTop,
    height: lineHeight,
  };
}

/**
 * Alma-style fake caret. Hides the textarea's native caret via CSS and
 * paints a smoothly-animated replacement that trails the cursor with a
 * comet tail.
 *
 * The component resolves the target `<textarea>` lazily by polling a
 * short interval at mount: pi-web-ui's composer is a Lit custom element
 * whose textarea lands in the DOM asynchronously, so we can't ref it
 * through React.
 */
export function AlmaCaret({ measureKey }: AlmaCaretProps = {}) {
  const [pos, setPos] = useState<CaretPos | null>(null);
  const [visible, setVisible] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // When ancestors animate (e.g. the composer slides out of welcome
  // position), `getBoundingClientRect()` reports the mid-animation
  // position at measure time — we re-measure once immediately and
  // again after the layout transition to land the caret at the final
  // resting place.
  useEffect(() => {
    if (measureKey === undefined) return;
    const ta = textareaRef.current;
    if (!ta) return;
    const now = measureCaret(ta);
    if (now) setPos(now);
    const timer = window.setTimeout(() => {
      const next = measureCaret(ta);
      if (next) setPos(next);
    }, 460);
    return () => window.clearTimeout(timer);
  }, [measureKey]);

  useEffect(() => {
    let raf = 0;
    let canceled = false;

    // Poll until pi-web-ui's textarea mounts, then bind listeners.
    const findAndBind = () => {
      if (canceled) return;
      const ta = document.querySelector<HTMLTextAreaElement>("textarea");
      if (!ta) {
        raf = window.setTimeout(findAndBind, 80);
        return;
      }
      textareaRef.current = ta;
      ta.style.caretColor = "transparent";
      // IME (composition) shows a browser-native caret under Chromium; accept it.

      const update = () => {
        const next = measureCaret(ta);
        if (next) setPos(next);
      };
      const focus = () => {
        setVisible(true);
        update();
      };
      const blur = () => setVisible(false);

      ta.addEventListener("input", update);
      ta.addEventListener("keydown", update);
      ta.addEventListener("keyup", update);
      ta.addEventListener("click", update);
      ta.addEventListener("scroll", update);
      ta.addEventListener("focus", focus);
      ta.addEventListener("blur", blur);
      window.addEventListener("resize", update);
      document.addEventListener("selectionchange", update);

      if (document.activeElement === ta) focus();
      update();

      return () => {
        ta.style.caretColor = "";
        ta.removeEventListener("input", update);
        ta.removeEventListener("keydown", update);
        ta.removeEventListener("keyup", update);
        ta.removeEventListener("click", update);
        ta.removeEventListener("scroll", update);
        ta.removeEventListener("focus", focus);
        ta.removeEventListener("blur", blur);
        window.removeEventListener("resize", update);
        document.removeEventListener("selectionchange", update);
      };
    };

    const cleanup = findAndBind();
    return () => {
      canceled = true;
      if (raf) clearTimeout(raf);
      if (typeof cleanup === "function") cleanup();
    };
  }, []);

  if (!pos || !visible) return null;

  // Head: sharp bar; smooth translate so moves across characters glide
  // rather than jumping. Blink animation still fires because it runs on
  // opacity, not transform.
  const head: React.CSSProperties = {
    position: "fixed",
    left: 0,
    top: 0,
    width: 2,
    height: pos.height,
    transform: `translate(${pos.x}px, ${pos.y}px)`,
    background: "var(--color-foreground)",
    transition: "transform 90ms cubic-bezier(0.2, 0.9, 0.2, 1)",
    animation: "alma-caret-blink 1.05s step-end infinite",
    pointerEvents: "none",
    zIndex: 30,
    willChange: "transform",
  };

  // Trail: painted only during moves. Keyed on the caret position so the
  // fade keyframe replays on every pixel change; between moves the element
  // sits at opacity 0 and disappears — no idle halo.
  const trail: React.CSSProperties = {
    position: "fixed",
    left: 0,
    top: 0,
    width: 24,
    height: pos.height,
    transform: `translate(${pos.x - 11}px, ${pos.y}px)`,
    background:
      "radial-gradient(ellipse at 55% 50%, color-mix(in oklab, oklch(0.62 0.18 250) 70%, transparent) 0%, transparent 70%)",
    transition: "transform 260ms cubic-bezier(0.25, 0.85, 0.25, 1)",
    filter: "blur(4px)",
    opacity: 0,
    animation: "alma-caret-trail 420ms ease-out",
    pointerEvents: "none",
    zIndex: 29,
    willChange: "transform, opacity",
  };

  return (
    <>
      <style>{`
        @keyframes alma-caret-blink { 0%, 55% { opacity: 1; } 56%, 100% { opacity: 0; } }
        @keyframes alma-caret-trail { 0% { opacity: 0; } 30% { opacity: 0.9; } 100% { opacity: 0; } }
      `}</style>
      <div
        aria-hidden
        key={`${Math.round(pos.x)}-${Math.round(pos.y)}`}
        style={trail}
      />
      <div aria-hidden style={head} />
    </>
  );
}
