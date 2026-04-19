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

import js from '@eslint/js'
import { defineConfig, globalIgnores } from 'eslint/config'
import importPlugin from 'eslint-plugin-import'
import jsxA11y from 'eslint-plugin-jsx-a11y'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import globals from 'globals'
import tseslint from 'typescript-eslint'

// NOTE: eslint-plugin-tailwindcss is intentionally NOT wired here.
// That plugin only supports Tailwind v3 and requires a JS config file
// (tailwind.config.{js,ts}) to resolve class lists. Rara is on Tailwind v4
// with CSS-first config (@theme in index.css), so the plugin throws
// "Cannot resolve default tailwindcss config path". Revisit once the
// plugin adds v4 support upstream.
//
// Several type-aware rules from `recommendedTypeChecked` and several
// `jsx-a11y` rules fire heavily on legacy pi-web-ui integration code.
// We demote those to warnings so the CI gate enforces the rules that
// catch real bugs (`no-floating-promises`, `no-misused-promises`,
// `import/order`) without blocking on pre-existing code smells. PR 2
// will tighten the ratchet as each warning category is cleaned up.

export default defineConfig([
  globalIgnores(['dist', 'playwright-report', 'coverage', '.vite', 'e2e', 'playwright.config.ts']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommendedTypeChecked,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
      jsxA11y.flatConfigs.recommended,
    ],
    plugins: {
      import: importPlugin,
    },
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-misused-promises': [
        'error',
        { checksVoidReturn: { attributes: false } },
      ],
      'import/order': [
        'error',
        {
          groups: ['builtin', 'external', 'internal', 'parent', 'sibling', 'index'],
          'newlines-between': 'always',
          alphabetize: { order: 'asc', caseInsensitive: true },
        },
      ],
      // Demote noisy type-aware / a11y / react rules to warnings — the
      // real enforcement happens for the three `error` rules above. PR 2
      // will ratchet these back up.
      '@typescript-eslint/no-unsafe-assignment': 'warn',
      '@typescript-eslint/no-unsafe-member-access': 'warn',
      '@typescript-eslint/no-unsafe-argument': 'warn',
      '@typescript-eslint/no-unsafe-return': 'warn',
      '@typescript-eslint/no-unsafe-call': 'warn',
      '@typescript-eslint/no-explicit-any': 'warn',
      '@typescript-eslint/no-unused-vars': 'warn',
      '@typescript-eslint/require-await': 'warn',
      '@typescript-eslint/no-base-to-string': 'warn',
      '@typescript-eslint/unbound-method': 'warn',
      '@typescript-eslint/restrict-template-expressions': 'warn',
      '@typescript-eslint/no-redundant-type-constituents': 'warn',
      '@typescript-eslint/only-throw-error': 'warn',
      'jsx-a11y/click-events-have-key-events': 'warn',
      'jsx-a11y/no-static-element-interactions': 'warn',
      'jsx-a11y/no-noninteractive-element-interactions': 'warn',
      'jsx-a11y/no-autofocus': 'warn',
      'react-refresh/only-export-components': 'warn',
      'react-hooks/set-state-in-effect': 'warn',
    },
  },
])
