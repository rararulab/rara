// @ts-nocheck
/* Vendor stub: @craft-agent/ui. Re-exports a small handful of helpers used across the
 * vendored chat + input tree. Real shapes live elsewhere; this is the minimum surface
 * needed for the BFS closure to compile. */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { type ReactNode } from 'react';

import { usePlatform as usePlatformReal } from '../../context/PlatformContext';

export type ActivityItem = any;
export type FileChange = any;
export type FilePreviewType = string;

export function classifyFile(_path: string): FilePreviewType {
  return 'unknown';
}

export function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

export const usePlatform = usePlatformReal;

export const CHAT_LAYOUT = {
  maxWidth: '720px',
  paddingX: '16px',
};

export const CHAT_CLASSES = {
  bubble: '',
  container: '',
};

export const Icon_Folder = (props: { className?: string }) => (
  <span className={props.className} aria-hidden>📁</span>
);

export const Icon_Home = (props: { className?: string }) => (
  <span className={props.className} aria-hidden>🏠</span>
);

export function Spinner(props: { className?: string }) {
  return <span className={props.className} aria-hidden>⏳</span>;
}

export function Tooltip({ children }: { children?: ReactNode }) {
  return <>{children}</>;
}
export function TooltipTrigger({ children }: { children?: ReactNode }) {
  return <>{children}</>;
}
export function TooltipContent({ children }: { children?: ReactNode }) {
  return <>{children}</>;
}
export function TooltipProvider({ children }: { children?: ReactNode }) {
  return <>{children}</>;
}

export function FilterableSelectPopover(_props: any): any {
  return null;
}

export function FileTypeIcon(props: { className?: string }) {
  return <span className={props.className} aria-hidden>📄</span>;
}

export function getFileTypeLabel(_path: string): string {
  return 'file';
}

const Passthrough = ({ children }: { children?: ReactNode }) => <>{children}</>;

export const DropdownMenu = Passthrough;
export const DropdownMenuTrigger = Passthrough;
export const DropdownMenuSub = Passthrough;
export const DropdownMenuShortcut = Passthrough;
export const StyledDropdownMenuContent = Passthrough;
export const StyledDropdownMenuItem = Passthrough;
export const StyledDropdownMenuSeparator = Passthrough;
export const StyledDropdownMenuSubTrigger = Passthrough;
export const StyledDropdownMenuSubContent = Passthrough;
