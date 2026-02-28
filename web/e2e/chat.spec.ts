import { test, expect } from '@playwright/test';

test.describe('Chat Page', () => {
  test.beforeEach(async ({ page }) => {
    // Mock the health endpoint so the app considers the server "online".
    // Without this, the ServerStatusProvider marks the server as offline,
    // which disables inputs and pauses TanStack Query.
    await page.route('**/api/v1/health', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: '{"status":"ok"}' }),
    );

    // Mock the sessions endpoint to return an empty list.
    await page.route('**/api/v1/chat/sessions*', (route) => {
      if (route.request().method() === 'GET') {
        return route.fulfill({ status: 200, contentType: 'application/json', body: '[]' });
      }
      // For POST (create session), return a fake session.
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          key: 'test-session-1',
          title: 'Test Session',
          model: 'openai/gpt-4o',
          message_count: 0,
          preview: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        }),
      });
    });

    // Mock the models endpoint.
    await page.route('**/api/v1/chat/models', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            id: 'openai/gpt-4o',
            name: 'GPT-4o',
            context_length: 128000,
            is_favorite: true,
          },
          {
            id: 'anthropic/claude-3.5-sonnet',
            name: 'Claude 3.5 Sonnet',
            context_length: 200000,
            is_favorite: false,
          },
        ]),
      }),
    );

    // The chat view is at /agent (default tab) or /agent?tab=chat.
    await page.goto('/agent?tab=chat');
  });

  test('renders the main chat layout', async ({ page }) => {
    // The Chat component's root is rendered inside the AgentConsole.
    // The session sidebar header contains a "Chat" button.
    const chatButton = page.getByRole('button', { name: 'Chat' });
    await expect(chatButton).toBeVisible();
  });

  test('shows the session sidebar with Chat / Operations tabs', async ({ page }) => {
    // The sidebar header has two tab buttons: "Chat" and "Operations".
    const chatTab = page.getByRole('button', { name: 'Chat' });
    const opsTab = page.getByRole('button', { name: 'Operations' });

    await expect(chatTab).toBeVisible();
    await expect(opsTab).toBeVisible();
  });

  test('shows empty state when no sessions exist', async ({ page }) => {
    // With the mocked empty sessions list, the session sidebar shows
    // "No conversations yet." along with instructions to start.
    const emptySessionHint = page.getByText('No conversations yet.');
    await expect(emptySessionHint).toBeVisible({ timeout: 10_000 });

    // The main area renders the EmptyState component which has a textarea
    // and send button (no separate heading text). Verify the textarea is
    // present as the primary empty-state indicator.
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 10_000 });
  });

  test('shows message input textarea', async ({ page }) => {
    // The EmptyState component renders a <textarea> for typing messages.
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 10_000 });
  });

  test('can type in the message input', async ({ page }) => {
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 10_000 });

    // With the mocked health endpoint, isOnline is true so the input is enabled.
    await textarea.fill('Hello, this is a test message');
    await expect(textarea).toHaveValue('Hello, this is a test message');
  });

  test('shows send button', async ({ page }) => {
    // With the server "online", the send button has title="Send message".
    const sendButton = page.getByRole('button', { name: 'Send message' });
    await expect(sendButton).toBeVisible({ timeout: 10_000 });
  });

  test('send button is disabled when input is empty', async ({ page }) => {
    const sendButton = page.getByRole('button', { name: 'Send message' });
    await expect(sendButton).toBeVisible({ timeout: 10_000 });
    await expect(sendButton).toBeDisabled();
  });

  test('send button becomes enabled when text is entered', async ({ page }) => {
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 10_000 });

    const sendButton = page.getByRole('button', { name: 'Send message' });
    await expect(sendButton).toBeDisabled();

    await textarea.fill('Hello');
    await expect(sendButton).toBeEnabled();
  });

  test('can switch to Operations view and back', async ({ page }) => {
    // Click "Operations" in the sidebar header.
    const opsButton = page.getByRole('button', { name: 'Operations' });
    await expect(opsButton).toBeVisible();
    await opsButton.click();

    // In Operations view, we should see tabs like "Status", "Tasks", "Scheduler".
    const statusTab = page.getByRole('button', { name: 'Status' });
    await expect(statusTab).toBeVisible({ timeout: 5_000 });

    // Switch back to Chat.
    const chatButton = page.getByRole('button', { name: 'Chat' });
    await chatButton.click();

    // Verify we're back on the chat view -- the textarea should be visible.
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 5_000 });
  });

  test('sidebar shows collapse toggle button', async ({ page }) => {
    // The session sidebar has a "Collapse conversations" button.
    const collapseBtn = page.getByRole('button', { name: /collapse conversations/i });
    await expect(collapseBtn).toBeVisible({ timeout: 5_000 });
  });

  test('textarea placeholder shows correct text when online', async ({ page }) => {
    const textarea = page.getByRole('textbox');
    await expect(textarea).toBeVisible({ timeout: 10_000 });
    await expect(textarea).toHaveAttribute(
      'placeholder',
      'Type a message... (Enter to send, Shift+Enter for newline)',
    );
  });

  test.skip('sends a message and receives a response', async ({ page }) => {
    // TODO: Requires a running backend with WebSocket support at
    // /api/v1/kernel/chat/ws. This test should:
    // 1. Type a message in the textarea
    // 2. Click the send button
    // 3. Wait for an assistant response to appear
    // 4. Verify the response bubble renders with markdown content
    await page.goto('/agent?tab=chat');
    const textarea = page.getByRole('textbox');
    await textarea.fill('Hello');
    const sendButton = page.getByRole('button', { name: 'Send message' });
    await sendButton.click();
    // Would need to wait for WebSocket response...
  });

  test.skip('creates a new chat session via dialog', async ({ page }) => {
    // TODO: Requires full backend integration to verify session creation
    // and appearance in the sidebar. The "New Conversation" dialog is
    // opened via a NewChatDialog component.
    // Steps:
    // 1. Open the new chat dialog
    // 2. Fill in a title
    // 3. Select a model
    // 4. Click "Create"
    // 5. Verify the new session appears in the sidebar
    await page.goto('/agent?tab=chat');
  });

  test.skip('deletes a chat session', async ({ page }) => {
    // TODO: Requires a running backend with existing sessions.
    // Steps:
    // 1. Hover over a session in the sidebar to reveal the delete button
    // 2. Click the delete button (title="Delete conversation")
    // 3. Verify the session is removed from the list
    await page.goto('/agent?tab=chat');
  });
});
