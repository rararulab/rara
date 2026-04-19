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

import AxeBuilder from '@axe-core/playwright';
import { test, expect } from '@playwright/test';

import { primeBackendUrl, stubApi } from './helpers';

test.describe('chat page with seeded sessions', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, {
      sessions: [
        {
          key: 'demo-one',
          title: 'Design review',
          preview: 'Let\u2019s look at the sidebar layout.',
          message_count: 3,
        },
        {
          key: 'demo-two',
          title: 'Kernel status',
          preview: 'Heartbeat is healthy.',
          message_count: 5,
        },
      ],
    });
    await primeBackendUrl(page);
  });

  test('renders the sidebar history list with rows and passes a11y', async ({ page }, testInfo) => {
    await page.goto('/');

    await expect(page.getByText('Design review')).toBeVisible();
    await expect(page.getByText('Kernel status')).toBeVisible();

    await expect(page).toHaveScreenshot(`chat-sessions-${testInfo.project.name}.png`, {
      fullPage: true,
      animations: 'disabled',
      mask: [page.locator('.animate-pulse')],
    });

    const axe = await new AxeBuilder({ page }).include('aside').analyze();
    expect(axe.violations, `axe violations: ${JSON.stringify(axe.violations, null, 2)}`).toEqual(
      [],
    );
  });

  test('sidebar new-session button is reachable', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByText('Design review')).toBeVisible();
    // Smoke: the primary action is present on every viewport. Focus
    // probing is skipped because Mobile Safari requires user-gesture
    // emulation to move focus onto a button.
    await expect(page.getByRole('button', { name: '新建会话' })).toBeEnabled();
  });
});
