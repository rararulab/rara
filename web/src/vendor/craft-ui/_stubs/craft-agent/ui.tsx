// @ts-nocheck
/* Vendor stub: @craft-agent/ui. Re-exports a small handful of helpers used across the
 * vendored chat + input tree. Real shapes live elsewhere; this is the minimum surface
 * needed for the BFS closure to compile. */
/* eslint-disable @typescript-eslint/no-explicit-any */
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

// Tooltip + dropdown surfaces are re-exported from the real radix-backed
// implementations vendored alongside this stub. The earlier `Passthrough`
// shims rendered every dropdown item as a flat sibling of the toolbar
// button (radix's open/closed gating was bypassed entirely), which made
// the model picker visually unusable. The stub still wins resolution for
// the `@craft-agent/ui` import — vite alias is set in `vite.config.ts` —
// but it now stitches through to the same components the rest of the
// vendor closure would import directly.
export {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
  TooltipProvider,
} from '../../components/tooltip';

export function FilterableSelectPopover(_props: any): any {
  return null;
}

export function FileTypeIcon(props: { className?: string }) {
  return <span className={props.className} aria-hidden>📄</span>;
}

export function getFileTypeLabel(_path: string): string {
  return 'file';
}

export {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuSub,
  DropdownMenuShortcut,
} from '../../components/electron-ui/dropdown-menu';

export {
  StyledDropdownMenuContent,
  StyledDropdownMenuItem,
  StyledDropdownMenuSeparator,
  StyledDropdownMenuSubTrigger,
  StyledDropdownMenuSubContent,
} from '../../components/ui/StyledDropdown';
