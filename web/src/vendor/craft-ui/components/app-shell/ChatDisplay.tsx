// @ts-nocheck
/* Vendor stub: ChatDisplay handle is referenced as a type/ref by EditPopover.
 * Rara's web shell does not mount ChatDisplay; we only need a placeholder. */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { forwardRef } from 'react';

export interface ChatDisplayHandle {
  scrollToBottom: () => void;
  focus: () => void;
  [key: string]: any;
}

export const ChatDisplay = forwardRef<ChatDisplayHandle, Record<string, unknown>>(function ChatDisplay() {
  return null;
});
