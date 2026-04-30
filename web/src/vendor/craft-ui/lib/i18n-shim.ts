/**
 * Minimal `react-i18next` stand-in for vendored craft-ui components.
 *
 * craft-agents-oss components import `useTranslation` from `react-i18next`
 * and pass i18n keys (e.g. `t('common.search')`) for default labels. rara
 * does not currently ship react-i18next; rather than introduce that whole
 * stack just for vendored defaults, we expose the same API surface but
 * return the key verbatim. Callers can override every label via props.
 *
 * If rara later adopts react-i18next properly, swap this shim for the real
 * import and the keys become live translations with no other code changes.
 */

export function useTranslation(): { t: (key: string) => string } {
  return { t: (key: string) => key };
}
