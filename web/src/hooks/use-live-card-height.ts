import { useEffect, useRef } from 'react';

/**
 * CSS custom property name written on the target element. The chat CSS
 * reads this from `<main>` to reserve scroll padding inside pi-web-ui's
 * message viewport equal to the live card's measured height, so the
 * user's just-sent message stays visible above the floating card.
 */
const CSS_VAR = '--rara-live-card-h';

/**
 * ResizeObserver-backed measurement of the in-progress agent live card.
 *
 * The live card is `position: absolute` and overlays the bottom of
 * pi-web-ui's message list (see `.rara-live-slot` in `index.css`).
 * Without compensation, pi-web-ui's auto-scroll lands the latest user
 * bubble directly under the card. This hook publishes the card's live
 * pixel height to a CSS variable on `targetRef`; the CSS rule then
 * pads pi-web-ui's scroll content by that amount, which (a) gives the
 * user bubble room to sit above the card and (b) — because pi-web-ui
 * watches its content's resize to drive auto-scroll — automatically
 * scrolls the bubble into view above the overlay.
 *
 * `cardRef` may be null (no active run); in that case the variable is
 * cleared so the chat returns to its normal layout.
 */
export function useLiveCardHeight(
  cardRef: React.RefObject<HTMLElement | null>,
  targetRef: React.RefObject<HTMLElement | null>,
) {
  // Track the last value we wrote so the cleanup can restore the prior
  // state instead of unconditionally wiping a value another caller set.
  const lastWrittenRef = useRef<string | null>(null);

  useEffect(() => {
    const target = targetRef.current;
    const card = cardRef.current;
    if (!target) return;

    const clear = () => {
      target.style.removeProperty(CSS_VAR);
      lastWrittenRef.current = null;
    };

    if (!card) {
      clear();
      return clear;
    }

    const write = (px: number) => {
      // Round to integer px — sub-pixel values cause layout thrash on
      // some browsers and the live card's height never genuinely needs
      // sub-pixel precision.
      const value = `${Math.max(0, Math.round(px))}px`;
      if (lastWrittenRef.current === value) return;
      target.style.setProperty(CSS_VAR, value);
      lastWrittenRef.current = value;
    };

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        // borderBoxSize is widely supported and matches `offsetHeight`
        // semantics (includes padding+border, excludes margin).
        const box = entry.borderBoxSize?.[0];
        const height = box ? box.blockSize : entry.contentRect.height;
        write(height);
      }
    });

    // Seed immediately so the first paint already has padding — without
    // this the first user message can flash under the card before the
    // observer fires.
    write(card.getBoundingClientRect().height);
    observer.observe(card);

    return () => {
      observer.disconnect();
      clear();
    };
  }, [cardRef, targetRef]);
}
