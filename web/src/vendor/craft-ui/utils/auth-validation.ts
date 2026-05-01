// @ts-nocheck
/* Vendor stub: pass-through credential validators. */
/* eslint-disable @typescript-eslint/no-explicit-any */
export function validateBasicAuthCredentials(_creds: any): { valid: boolean; error: string | null } {
  return { valid: true, error: null };
}

export function getPasswordValue(creds: any): string {
  return creds?.password ?? '';
}

export function getPasswordLabel(_creds: any): string {
  return 'Password';
}

export function getPasswordPlaceholder(_creds: any): string {
  return '';
}
