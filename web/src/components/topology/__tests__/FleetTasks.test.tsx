/**
 * BDD bindings for `specs/issue-2197-fleet-inbox-ui.spec.md`.
 *
 * Each `it(...)` name carries the spec's `Test:` selector verbatim so
 * `agent-spec lifecycle` can resolve scenarios to real test functions.
 * The HTTP layer is mocked — the read API (issue 2195) is not required
 * to run these.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { WorkerInbox } from '../WorkerInbox';

import type { FleetTask } from '@/api/fleet';

// --- Mocks -----------------------------------------------------------------

const apiGetMock = vi.fn();

vi.mock('@/api/client', () => ({
  api: {
    get: (...args: unknown[]) => apiGetMock(...args),
  },
}));

// --- Fixture helpers -------------------------------------------------------

function makeTask(partial: Partial<FleetTask> & { id: string }): FleetTask {
  return {
    id: partial.id,
    agent_type: partial.agent_type ?? 'claude-code',
    status: partial.status ?? 'running',
    backend: partial.backend ?? 'local-process',
    prompt: partial.prompt ?? `task ${partial.id}`,
    repo_url: partial.repo_url ?? 'https://github.com/rararulab/rara',
    session_key: partial.session_key ?? null,
    branch: partial.branch ?? null,
    commit_sha: partial.commit_sha ?? null,
    pr_url: partial.pr_url ?? null,
    input_tokens: partial.input_tokens ?? null,
    output_tokens: partial.output_tokens ?? null,
    cost_usd: partial.cost_usd ?? null,
    error: partial.error ?? null,
    exit_code: partial.exit_code ?? null,
    created_at: partial.created_at ?? '2026-07-05T00:00:00Z',
    started_at: partial.started_at ?? null,
    completed_at: partial.completed_at ?? null,
  };
}

function renderInbox() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: 0 },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <WorkerInbox
        rootSessionKey="root"
        events={[]}
        activeChildSession={null}
        onSelectChild={vi.fn()}
      />
    </QueryClientProvider>,
  );
}

/** Matches a lone (unpaired) UTF-16 surrogate — the artifact a naive
 *  `String.slice` truncation leaves behind when it splits a pair. */
const LONE_SURROGATE = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

// --- Setup -----------------------------------------------------------------

beforeEach(() => {
  apiGetMock.mockReset();
});

afterEach(() => {
  cleanup();
});

// --- Spec scenarios --------------------------------------------------------

describe('FleetTasks — worker-inbox fleet section (issue-2197)', () => {
  it('renders_fleet_tasks_from_api', async () => {
    apiGetMock.mockResolvedValue([
      makeTask({ id: 't1', agent_type: 'claude-code', status: 'running' }),
      makeTask({
        id: 't2',
        agent_type: 'codex',
        status: 'succeeded',
        completed_at: '2026-07-05T01:00:00Z',
      }),
    ]);

    renderInbox();

    await waitFor(() => {
      expect(screen.getByText('Fleet tasks')).toBeInTheDocument();
    });
    expect(screen.getByText('claude-code')).toBeInTheDocument();
    expect(screen.getByText('running')).toBeInTheDocument();
    expect(screen.getByText('codex')).toBeInTheDocument();
    expect(screen.getByText('succeeded')).toBeInTheDocument();
    expect(apiGetMock).toHaveBeenCalledWith('/api/v1/fleet/tasks', expect.anything());
  });

  it('terminal_task_shows_pr_link_and_cost', async () => {
    apiGetMock.mockResolvedValue([
      makeTask({
        id: 't1',
        status: 'succeeded',
        branch: 'issue-42-fix-flake',
        pr_url: 'https://github.com/rararulab/rara/pull/123',
        input_tokens: 8000,
        output_tokens: 4500,
        cost_usd: 0.42,
        completed_at: '2026-07-05T01:00:00Z',
      }),
    ]);

    renderInbox();

    await waitFor(() => {
      expect(screen.getByText('issue-42-fix-flake')).toBeInTheDocument();
    });
    expect(screen.getByRole('link')).toHaveAttribute(
      'href',
      'https://github.com/rararulab/rara/pull/123',
    );
    expect(screen.getByText('12.5k tok')).toBeInTheDocument();
    expect(screen.getByText('$0.42')).toBeInTheDocument();
  });

  it('failed_task_shows_error', async () => {
    apiGetMock.mockResolvedValue([
      makeTask({
        id: 't1',
        status: 'failed',
        error: 'agent exited with code 3: quality gate failed',
        exit_code: 3,
        completed_at: '2026-07-05T01:00:00Z',
      }),
    ]);

    renderInbox();

    await waitFor(() => {
      expect(screen.getByText('failed')).toBeInTheDocument();
    });
    expect(screen.getByText('agent exited with code 3: quality gate failed')).toBeInTheDocument();
  });

  // Regression guard beyond the spec scenarios: pr_url comes from the
  // worker result callback (untrusted LLM/worker output), so non-http(s)
  // values must never become an anchor.
  it('unsafe_pr_url_renders_as_plain_text', async () => {
    apiGetMock.mockResolvedValue([
      makeTask({
        id: 't1',
        status: 'succeeded',
        pr_url: 'javascript:alert(1)',
        completed_at: '2026-07-05T01:00:00Z',
      }),
    ]);

    renderInbox();

    await waitFor(() => {
      expect(screen.getByText('javascript:alert(1)')).toBeInTheDocument();
    });
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(document.querySelector('a')).toBeNull();
  });

  it('hides_fleet_section_when_empty', async () => {
    apiGetMock.mockResolvedValue([]);

    renderInbox();

    await waitFor(() => {
      expect(apiGetMock).toHaveBeenCalled();
    });
    expect(screen.queryByText('Fleet tasks')).not.toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Fleet tasks' })).not.toBeInTheDocument();
    // The worker empty state is unrelated to fleet tasks and stays.
    expect(screen.getByText('No workers spawned yet.')).toBeInTheDocument();
  });

  it('cjk_prompt_truncates_safely', async () => {
    // 'a' + repeated CJK Extension B char (surrogate pair in UTF-16):
    // a naive UTF-16 `slice` lands mid-pair and leaves a lone surrogate.
    const prompt = `a${'𠀋汉字调查报告'.repeat(40)}`;
    apiGetMock.mockResolvedValue([makeTask({ id: 't1', prompt })]);

    renderInbox();

    await waitFor(() => {
      expect(screen.getByTitle(prompt)).toBeInTheDocument();
    });
    const rendered = screen.getByTitle(prompt).textContent ?? '';
    expect(rendered.endsWith('…')).toBe(true);
    const body = rendered.slice(0, -1);
    expect(body).not.toMatch(LONE_SURROGATE);
    expect(body).not.toContain('�');
    // The preview is a true prefix of the original prompt, truncated by
    // code point.
    expect(prompt.startsWith(body)).toBe(true);
    expect(Array.from(body)).toHaveLength(80);
  });
});
