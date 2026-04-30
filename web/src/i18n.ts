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

import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

// Vendor (`craft-agents-oss`) sprinkles `useTranslation()` throughout the
// chat + input tree without owning the i18next bootstrap. We don't ship
// translations of vendor strings, but raw keys like `chat.attachFiles`
// surfacing in the toolbar are noisy. Convert the last dotted segment to
// a humanised label on the fly — `chat.attachFiles` → "Attach files",
// `thinking.notSupported` → "Not supported". Cheap, no per-key data to
// maintain as vendor evolves.
function humaniseKey(key: string): string {
  const tail = key.split('.').pop() ?? key;
  // Split camelCase / PascalCase / snake_case / kebab-case, lowercase the
  // run, then capitalise the first letter of the whole label so it reads
  // like "Attach files" rather than "attach Files".
  const spaced = tail
    .replace(/[_-]+/g, ' ')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .toLowerCase()
    .trim();
  if (!spaced) return key;
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

void i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} } },
  interpolation: { escapeValue: false },
  // Vendor keys are dotted (e.g. "input.placeholder.default"); without
  // this, i18next would interpret the dots as namespace/sub-key splits
  // and hide the fallback behind nested-object lookups.
  keySeparator: false,
  nsSeparator: false,
  parseMissingKeyHandler: humaniseKey,
});

export default i18n;
