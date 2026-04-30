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
// Those rules are turned OFF here (not `warn`) so that the CI gate
// `eslint --max-warnings=0` can enforce everything else without
// dragging in ~94 pre-existing violations. Each `off` rule carries a
// TODO(#1606) pointer — #1606 tracks ratcheting these back to `error`
// one rule at a time as the underlying violations are cleaned up.
// Leaving rules at `warn` was tried first and rejected: warnings get
// ignored, and `--max-warnings=0` would block every PR on unrelated
// legacy code.

export default defineConfig([
  globalIgnores([
    'dist',
    'playwright-report',
    'coverage',
    '.vite',
    'e2e',
    'playwright.config.ts',
    // Vendored craft-ui sources are excluded from the typed project
    // (see web/tsconfig.app.json `exclude`). ESLint's project service
    // therefore can't load them — skip the entire tree from lint.
    'src/vendor/**',
  ]),
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
      // Rules turned OFF and tracked in #1606 — noisy on legacy code,
      // ratchet back to `error` one rule at a time.
      // TODO(#1606): ratchet these to error once violations are cleaned up.
      '@typescript-eslint/no-unsafe-assignment': 'off',
      '@typescript-eslint/no-unsafe-member-access': 'off',
      '@typescript-eslint/no-unsafe-argument': 'off',
      '@typescript-eslint/no-unsafe-return': 'off',
      '@typescript-eslint/no-unsafe-call': 'off',
      '@typescript-eslint/no-explicit-any': 'off',
      '@typescript-eslint/require-await': 'off',
      '@typescript-eslint/no-base-to-string': 'off',
      '@typescript-eslint/unbound-method': 'off',
      '@typescript-eslint/restrict-template-expressions': 'off',
      '@typescript-eslint/no-redundant-type-constituents': 'off',
      '@typescript-eslint/only-throw-error': 'off',
      'jsx-a11y/click-events-have-key-events': 'off',
      'jsx-a11y/no-static-element-interactions': 'off',
      'jsx-a11y/no-noninteractive-element-interactions': 'off',
      'jsx-a11y/no-autofocus': 'off',
      'react-refresh/only-export-components': 'off',
      'react-hooks/set-state-in-effect': 'off',
      // Fixed in-place rather than deferred: leading-underscore convention
      // is used project-wide to mark intentionally-unused params.
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
      ],
    },
  },
])
