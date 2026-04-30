// @ts-nocheck
/* Vendor stub: jotai session atom family. Returns an empty session metadata. */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { atom } from 'jotai';

export interface SessionMeta {
  id: string;
  title: string;
  status: string;
  [key: string]: any;
}

const EMPTY_SESSION: SessionMeta = { id: '', title: '', status: 'idle' };

export function sessionAtomFamily(_id: string) {
  return atom<SessionMeta>(EMPTY_SESSION);
}
