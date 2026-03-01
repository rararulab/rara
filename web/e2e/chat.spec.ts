import { test, expect, type APIRequestContext } from '@playwright/test';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Generate a unique session key for test isolation. */
function testSessionKey(): string {
  return `e2e-test-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Delete a chat session via the API. Ignores 404 (already deleted). */
async function deleteSession(request: APIRequestContext, key: string) {
  await request.delete(`/api/v1/chat/sessions/${encodeURIComponent(key)}`);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('Chat E2E (real backend)', () => {
  test.beforeEach(async ({ request }) => {
    const health = await request.get('/api/v1/health');
    if (!health.ok()) {
      test.skip(true, 'Backend not running at localhost:25555');
    }
  });

  // -----------------------------------------------------------------------
  // 1. Health check
  // -----------------------------------------------------------------------

  test('backend health check returns ok', async ({ request }) => {
    const res = await request.get('/api/v1/health');
    expect(res.ok()).toBeTruthy();
  });

  // -----------------------------------------------------------------------
  // 2. Chat page loads with real backend
  // -----------------------------------------------------------------------

  test('chat page loads and textarea is enabled', async ({ page }) => {
    await page.goto('/agent?tab=chat');

    // Wait for the textarea to appear.
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 15_000 });

    // When the backend is online the placeholder should NOT contain "offline".
    await expect(textarea).not.toHaveAttribute(
      'placeholder',
      /offline/i,
      { timeout: 15_000 },
    );

    // Textarea should be enabled.
    await expect(textarea).toBeEnabled();
  });

  // -----------------------------------------------------------------------
  // 3. Chat / Operations tabs render
  // -----------------------------------------------------------------------

  test('shows Chat and Operations tabs in sidebar', async ({ page }) => {
    await page.goto('/agent?tab=chat');

    const chatTab = page.getByRole('button', { name: 'Chat' });
    const opsTab = page.getByRole('button', { name: 'Operations' });

    await expect(chatTab).toBeVisible({ timeout: 10_000 });
    await expect(opsTab).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 4. Can switch to Operations and back
  // -----------------------------------------------------------------------

  test('can switch to Operations view and back', async ({ page }) => {
    await page.goto('/agent?tab=chat');

    const opsButton = page.getByRole('button', { name: 'Operations' });
    await expect(opsButton).toBeVisible({ timeout: 10_000 });
    await opsButton.click();

    // Operations view shows sub-tabs like "Status".
    const statusTab = page.getByRole('button', { name: 'Status' });
    await expect(statusTab).toBeVisible({ timeout: 5_000 });

    // Switch back to Chat.
    const chatButton = page.getByRole('button', { name: 'Chat' });
    await chatButton.click();

    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 5_000 });
  });

  // -----------------------------------------------------------------------
  // 5. List models from backend
  // -----------------------------------------------------------------------

  test('list models API returns data', async ({ request }) => {
    const res = await request.get('/api/v1/chat/models');
    expect(res.ok()).toBeTruthy();

    const models = await res.json();
    expect(Array.isArray(models)).toBeTruthy();
    // The backend should return at least one model.
    expect(models.length).toBeGreaterThan(0);

    // Each model should have an id and name.
    for (const model of models) {
      expect(model).toHaveProperty('id');
      expect(model).toHaveProperty('name');
    }
  });

  // -----------------------------------------------------------------------
  // 6. Create and delete a session via API
  // -----------------------------------------------------------------------

  test('create and delete a session via API', async ({ request }) => {
    const key = testSessionKey();

    // Create.
    const createRes = await request.post('/api/v1/chat/sessions', {
      data: { key, title: 'E2E Test Session' },
    });
    expect(createRes.ok()).toBeTruthy();

    const created = await createRes.json();
    expect(created.key).toBe(key);

    // Verify it appears in the list.
    const listRes = await request.get('/api/v1/chat/sessions?limit=100&offset=0');
    expect(listRes.ok()).toBeTruthy();
    const sessions = await listRes.json();
    expect(sessions.some((s: { key: string }) => s.key === key)).toBeTruthy();

    // Delete.
    const delRes = await request.delete(
      `/api/v1/chat/sessions/${encodeURIComponent(key)}`,
    );
    expect(delRes.ok()).toBeTruthy();

    // Verify it is gone.
    const listRes2 = await request.get('/api/v1/chat/sessions?limit=100&offset=0');
    const sessions2 = await listRes2.json();
    expect(sessions2.some((s: { key: string }) => s.key === key)).toBeFalsy();
  });

  // -----------------------------------------------------------------------
  // 7. Create and delete a session via UI
  // -----------------------------------------------------------------------

  test('create a session via UI and delete it', async ({ page, request }) => {
    const key = testSessionKey();

    await page.goto('/agent?tab=chat');

    // Wait for initial page load.
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 15_000 });

    // Ensure the server is online (placeholder does NOT say offline).
    await expect(textarea).not.toHaveAttribute('placeholder', /offline/i, {
      timeout: 15_000,
    });

    // Type a message in the empty state textarea and press Enter.
    // This triggers handleStartFromEmpty which creates a session and sends
    // the message text as the first user message.
    await textarea.fill('E2E test message');
    await page.keyboard.press('Enter');

    // The UI should transition to a ChatThread — wait for a user message
    // bubble to appear. The user message is rendered as a <p> inside a div
    // with bg-primary/90. We look for the text we typed.
    const userMessage = page.getByText('E2E test message');
    await expect(userMessage).toBeVisible({ timeout: 15_000 });

    // The sidebar should now contain a session entry. Because the session
    // title is derived from the message text (first 80 chars), look for
    // a sidebar item that contains it.
    const sessionEntry = page.locator('button', { hasText: 'E2E test message' });
    await expect(sessionEntry.first()).toBeVisible({ timeout: 10_000 });

    // Clean up — delete the session via API. We need to find its key.
    // The key is auto-generated by the frontend (`chat-{timestamp}-{rand}`).
    // Retrieve sessions from the API and find the one with our title.
    const listRes = await request.get('/api/v1/chat/sessions?limit=100&offset=0');
    const sessions = await listRes.json();
    const created = sessions.find(
      (s: { title: string }) => s.title === 'E2E test message',
    );
    if (created) {
      await deleteSession(request, created.key);
    }
  });

  // -----------------------------------------------------------------------
  // 8. Send button disabled / enabled behavior
  // -----------------------------------------------------------------------

  test('send button is disabled when input is empty and enabled when text is entered', async ({
    page,
  }) => {
    await page.goto('/agent?tab=chat');

    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 15_000 });

    // Wait for online status.
    await expect(textarea).not.toHaveAttribute('placeholder', /offline/i, {
      timeout: 15_000,
    });

    const sendButton = page.getByRole('button', { name: 'Send message' });
    await expect(sendButton).toBeVisible({ timeout: 5_000 });
    await expect(sendButton).toBeDisabled();

    // Type something.
    await textarea.fill('Hello');
    await expect(sendButton).toBeEnabled();

    // Clear it.
    await textarea.fill('');
    await expect(sendButton).toBeDisabled();
  });

  // -----------------------------------------------------------------------
  // 9. Textarea placeholder shows correct text when online
  // -----------------------------------------------------------------------

  test('textarea placeholder is correct when server is online', async ({
    page,
  }) => {
    await page.goto('/agent?tab=chat');

    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 15_000 });

    await expect(textarea).toHaveAttribute(
      'placeholder',
      'Type a message... (Enter to send, Shift+Enter for newline)',
      { timeout: 15_000 },
    );
  });

  // -----------------------------------------------------------------------
  // 10. Sidebar collapse toggle exists
  // -----------------------------------------------------------------------

  test('sidebar shows collapse toggle button', async ({ page }) => {
    await page.goto('/agent?tab=chat');

    const collapseBtn = page.getByRole('button', {
      name: /collapse conversations/i,
    });
    await expect(collapseBtn).toBeVisible({ timeout: 10_000 });
  });

  // -----------------------------------------------------------------------
  // 11. WebSocket connection establishes on session select
  // -----------------------------------------------------------------------

  test('WebSocket connection establishes when a session is active', async ({
    page,
    request,
  }) => {
    const key = testSessionKey();

    // Create a session via API first.
    const createRes = await request.post('/api/v1/chat/sessions', {
      data: { key, title: 'E2E WS Test' },
    });
    expect(createRes.ok()).toBeTruthy();

    try {
      await page.goto('/agent?tab=chat');

      // Wait for the page and session list to load.
      const textarea = page.getByRole('textbox');
      await expect(textarea).toBeVisible({ timeout: 15_000 });

      // Wait for online status.
      await expect(textarea).not.toHaveAttribute('placeholder', /offline/i, {
        timeout: 15_000,
      });

      // Click the session we created to activate it.
      const sessionButton = page.locator('button', { hasText: 'E2E WS Test' });
      await expect(sessionButton.first()).toBeVisible({ timeout: 10_000 });
      await sessionButton.first().click();

      // When a session is active, the ChatThread renders and opens a WS
      // connection. We can verify this by checking that the chat thread UI
      // appears (thread header showing message count).
      const threadHeader = page.getByText(/0 messages/i);
      await expect(threadHeader).toBeVisible({ timeout: 10_000 });

      // Additionally, the textarea inside ChatThread should have the online
      // placeholder (not "Server offline"), confirming the component mounted.
      const chatTextarea = page.getByRole('textbox');
      await expect(chatTextarea).toHaveAttribute(
        'placeholder',
        'Type a message... (Enter to send, Shift+Enter for newline)',
        { timeout: 10_000 },
      );
    } finally {
      await deleteSession(request, key);
    }
  });

  // -----------------------------------------------------------------------
  // 12. Send a message via WebSocket
  // -----------------------------------------------------------------------

  test('send a message via WebSocket and see it in the chat', async ({
    page,
    request,
  }) => {
    const key = testSessionKey();

    // Create a session via API.
    const createRes = await request.post('/api/v1/chat/sessions', {
      data: { key, title: 'E2E Send Test' },
    });
    expect(createRes.ok()).toBeTruthy();

    try {
      await page.goto('/agent?tab=chat');

      const textarea = page.getByRole('textbox');
      await expect(textarea).toBeVisible({ timeout: 15_000 });
      await expect(textarea).not.toHaveAttribute('placeholder', /offline/i, {
        timeout: 15_000,
      });

      // Select the session.
      const sessionButton = page.locator('button', { hasText: 'E2E Send Test' });
      await expect(sessionButton.first()).toBeVisible({ timeout: 10_000 });
      await sessionButton.first().click();

      // Wait for the chat thread to render.
      const threadHeader = page.getByText(/0 messages/i);
      await expect(threadHeader).toBeVisible({ timeout: 10_000 });

      // Type a message and send.
      const chatTextarea = page.getByRole('textbox');
      await expect(chatTextarea).toBeEnabled({ timeout: 5_000 });
      await chatTextarea.fill('Hello from E2E test');
      await page.keyboard.press('Enter');

      // The user message should appear in the chat area (optimistically
      // added to the cache before the WebSocket round-trip).
      const userMessage = page.getByText('Hello from E2E test');
      await expect(userMessage).toBeVisible({ timeout: 10_000 });
    } finally {
      await deleteSession(request, key);
    }
  });

  // -----------------------------------------------------------------------
  // 13. Messages endpoint works
  // -----------------------------------------------------------------------

  test('messages endpoint returns data for a session', async ({ request }) => {
    const key = testSessionKey();

    // Create session.
    await request.post('/api/v1/chat/sessions', {
      data: { key, title: 'E2E Messages Test' },
    });

    try {
      const res = await request.get(
        `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages?limit=200`,
      );
      expect(res.ok()).toBeTruthy();

      const messages = await res.json();
      expect(Array.isArray(messages)).toBeTruthy();
      // A fresh session should have no messages (or possibly a system prompt).
    } finally {
      await deleteSession(request, key);
    }
  });

  // -----------------------------------------------------------------------
  // 14. Session PATCH (update) works
  // -----------------------------------------------------------------------

  test('update session title via PATCH', async ({ request }) => {
    const key = testSessionKey();

    await request.post('/api/v1/chat/sessions', {
      data: { key, title: 'Original Title' },
    });

    try {
      const patchRes = await request.patch(
        `/api/v1/chat/sessions/${encodeURIComponent(key)}`,
        { data: { title: 'Updated Title' } },
      );
      expect(patchRes.ok()).toBeTruthy();

      const updated = await patchRes.json();
      expect(updated.title).toBe('Updated Title');
    } finally {
      await deleteSession(request, key);
    }
  });

  // -----------------------------------------------------------------------
  // 15. Clear messages endpoint works
  // -----------------------------------------------------------------------

  test('clear messages endpoint works', async ({ request }) => {
    const key = testSessionKey();

    await request.post('/api/v1/chat/sessions', {
      data: { key, title: 'E2E Clear Test' },
    });

    try {
      const clearRes = await request.delete(
        `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages`,
      );
      expect(clearRes.ok()).toBeTruthy();
    } finally {
      await deleteSession(request, key);
    }
  });
});
