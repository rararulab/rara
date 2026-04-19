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

test.describe('welcome page', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, { sessions: [] });
    await primeBackendUrl(page);
  });

  test('renders without active sessions and passes baseline a11y', async ({ page }, testInfo) => {
    await page.goto('/');

    // Wait for the sidebar history pane to settle into its empty state.
    await expect(page.getByText('暂无会话')).toBeVisible();
    await expect(page.getByRole('button', { name: '新建会话' })).toBeVisible();

    await expect(page).toHaveScreenshot(`welcome-${testInfo.project.name}.png`, {
      fullPage: true,
      animations: 'disabled',
      mask: [page.locator('.animate-pulse')],
    });

    const axe = await new AxeBuilder({ page })
      // pi-web-ui internals inject styles without a matching manifest —
      // we scope axe to the sidebar we own and can control.
      .include('aside')
      .analyze();
    expect(axe.violations, `axe violations: ${JSON.stringify(axe.violations, null, 2)}`).toEqual(
      [],
    );
  });
});
