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
// translations of vendor strings — falling back to the i18n key text is
// acceptable while we don't have a localisation surface — but the hook
// still requires an initialised instance, otherwise the first vendor
// render warns and any consumer reading `t(...)` ends up with `undefined`.
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
});

export default i18n;
