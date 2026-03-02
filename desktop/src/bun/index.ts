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

import Electrobun, { BrowserWindow } from "electrobun/bun";
import path from "node:path";

type ChildProc = ReturnType<typeof Bun.spawn>;

const apiPort = Number.parseInt(process.env.RARA_DESKTOP_API_PORT ?? "25555", 10);
const webPort = Number.parseInt(process.env.RARA_DESKTOP_WEB_PORT ?? "5173", 10);
const repoRoot = process.env.RARA_DESKTOP_REPO_ROOT
  ? path.resolve(process.env.RARA_DESKTOP_REPO_ROOT)
  : null;
const shellTitle = "Rara Desktop";
const startBackend = process.env.RARA_DESKTOP_START_BACKEND === "1";
const startFrontend = process.env.RARA_DESKTOP_START_FRONTEND === "1";
const managedMode = startBackend || startFrontend;

const children: ChildProc[] = [];

function envWithRepo(): NodeJS.ProcessEnv {
  return {
    ...process.env,
    RARA_DESKTOP: "1",
  };
}

function spawnLogged(
  name: string,
  cmd: string[],
  cwd: string,
): ChildProc {
  console.log(`[desktop] spawning ${name}: ${cmd.join(" ")} (cwd=${cwd})`);
  const child = Bun.spawn(cmd, {
    cwd,
    env: envWithRepo(),
    stdout: "pipe",
    stderr: "pipe",
  });
  children.push(child);
  void pipeOutput(name, "stdout", child.stdout);
  void pipeOutput(name, "stderr", child.stderr);
  void child.exited.then((code) => {
    console.log(`[desktop] ${name} exited with code ${code}`);
  });
  return child;
}

async function pipeOutput(
  name: string,
  streamName: "stdout" | "stderr",
  stream: ReadableStream<Uint8Array> | null,
): Promise<void> {
  if (!stream) return;
  const decoder = new TextDecoder();
  try {
    for await (const chunk of stream) {
      const text = decoder.decode(chunk, { stream: true });
      const lines = text.split(/\r?\n/);
      for (const line of lines) {
        if (!line) continue;
        const prefix = streamName === "stderr" ? "!" : ">";
        console.log(`[${name}] ${prefix} ${line}`);
      }
    }
  } catch (error) {
    console.warn(`[desktop] failed reading ${name} ${streamName}`, error);
  }
}

async function waitForHttp(url: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown = null;

  while (Date.now() < deadline) {
    try {
      const resp = await fetch(url);
      if (resp.ok) return;
      lastError = new Error(`HTTP ${resp.status}`);
    } catch (error) {
      lastError = error;
    }
    await Bun.sleep(500);
  }

  throw new Error(`Timed out waiting for ${url}: ${String(lastError ?? "unknown error")}`);
}

function isSettingsUrl(rawUrl: string): boolean {
  try {
    const url = new URL(rawUrl, `http://127.0.0.1:${webPort}`);
    return url.pathname === "/settings";
  } catch {
    return false;
  }
}

function extractEventUrl(event: unknown): string {
  if (!event || typeof event !== "object") return "";
  const e = event as {
    data?: { detail?: unknown; url?: unknown };
    detail?: unknown;
  };

  if (typeof e.data?.url === "string") return e.data.url;
  if (typeof e.data?.detail === "string") return e.data.detail;
  if (typeof e.detail === "string") return e.detail;

  const dataDetail = e.data?.detail;
  if (dataDetail && typeof dataDetail === "object" && "url" in dataDetail) {
    const candidate = (dataDetail as { url?: unknown }).url;
    if (typeof candidate === "string") return candidate;
  }

  const detail = e.detail;
  if (detail && typeof detail === "object" && "url" in detail) {
    const candidate = (detail as { url?: unknown }).url;
    if (typeof candidate === "string") return candidate;
  }

  return "";
}

function tryBlockNavigation(event: unknown): void {
  if (!event || typeof event !== "object") return;
  const e = event as { response?: unknown };
  try {
    e.response = { allow: false };
  } catch {
    // some event types may not support response mutation
  }
}

function attachWindowOpenHandlers(win: BrowserWindow): void {
  win.webview.on("new-window-open", (event: unknown) => {
    const rawUrl = extractEventUrl(event);

    if (!rawUrl) return;

    if (isSettingsUrl(rawUrl)) {
      tryBlockNavigation(event);
      const child = createAppWindow(rawUrl, {
        width: 1100,
        height: 860,
        title: "Rara Settings",
      });
      child.focus();
    }
  });

  win.webview.on("will-navigate", (event: unknown) => {
    const rawUrl = extractEventUrl(event);
    if (!rawUrl || !isSettingsUrl(rawUrl)) return;

    tryBlockNavigation(event);
    const child = createAppWindow(rawUrl, {
      width: 1100,
      height: 860,
      title: "Rara Settings",
    });
    child.focus();
  });
}

function createAppWindow(
  url: string,
  opts?: { width?: number; height?: number; title?: string },
): BrowserWindow {
  const win = new BrowserWindow({
    title: opts?.title ?? shellTitle,
    url,
    frame: {
      x: 80,
      y: 80,
      width: opts?.width ?? 1440,
      height: opts?.height ?? 900,
    },
    titleBarStyle: "default",
    sandbox: true,
  });
  attachWindowOpenHandlers(win);
  return win;
}

async function shutdownChildren(): Promise<void> {
  for (const child of children) {
    try {
      child.kill();
    } catch {
      // ignore; process may already be gone
    }
  }

  await Promise.allSettled(
    children.map(async (child) => {
      const timeout = Bun.sleep(2_000).then(() => "timeout");
      const exited = child.exited.then(() => "exited");
      const result = await Promise.race([timeout, exited]);
      if (result === "timeout") {
        try {
          child.kill("SIGKILL");
        } catch {
          // ignore
        }
      }
    }),
  );
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function createErrorWindow(error: unknown): void {
  const expectedCommands = repoRoot
    ? `cd ${repoRoot}\njust run\njust web`
    : "cd <repo-root>\njust run\njust web";
  const errorText = escapeHtml(String(error));
  const modeLabel = managedMode ? "Managed Dev Mode" : "Split Mode";
  const modeHint = managedMode
    ? "Desktop shell attempted to start local services and one of them did not become ready."
    : "Desktop shell started, but the target frontend/backend endpoints are not reachable yet.";
  const nextSteps = managedMode
    ? "Check terminal logs below the Electrobun process (backend/frontend child process output is prefixed)."
    : "Start services separately (`just run` and `just web`), then relaunch `just desktop`.";
  const commandBlock = escapeHtml(
    managedMode ? expectedCommands : "just run   # terminal 1\njust web   # terminal 2\njust desktop",
  );
  const safeModeLabel = escapeHtml(modeLabel);
  const safeModeHint = escapeHtml(modeHint);
  const safeNextSteps = escapeHtml(nextSteps);
  const safeRepoRoot = escapeHtml(repoRoot ?? "(not set)");
  const html = `
    <!doctype html>
    <html>
      <head>
        <meta charset="utf-8" />
        <title>${shellTitle} startup failed</title>
        <style>
          :root {
            --bg0: #06131a;
            --bg1: #0a1c25;
            --panel: rgba(11, 24, 34, 0.9);
            --panel-2: rgba(17, 31, 43, 0.9);
            --border: rgba(167, 203, 222, 0.18);
            --text: #e8f1f5;
            --muted: #9ab0bd;
            --accent: #4fd1c5;
            --warn: #ffb86b;
            --danger: #ff6b6b;
            --shadow: 0 20px 60px rgba(0,0,0,0.35);
          }
          * { box-sizing: border-box; }
          html, body { height: 100%; margin: 0; }
          body {
            font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "SF Pro Display", "Segoe UI", sans-serif;
            color: var(--text);
            background:
              radial-gradient(1200px 500px at 15% -10%, rgba(79,209,197,0.12), transparent 60%),
              radial-gradient(900px 500px at 100% 0%, rgba(255,184,107,0.10), transparent 55%),
              linear-gradient(180deg, var(--bg0), var(--bg1));
            padding: 28px;
          }
          .shell {
            width: min(1100px, 100%);
            margin: 0 auto;
            background: linear-gradient(180deg, rgba(255,255,255,0.02), rgba(255,255,255,0.00));
            border: 1px solid var(--border);
            border-radius: 18px;
            overflow: hidden;
            box-shadow: var(--shadow);
            backdrop-filter: blur(8px);
          }
          .hero {
            padding: 18px 20px;
            border-bottom: 1px solid var(--border);
            background: linear-gradient(180deg, rgba(79,209,197,0.08), rgba(79,209,197,0.01));
          }
          .badge {
            display: inline-flex;
            align-items: center;
            gap: 8px;
            border: 1px solid rgba(255,107,107,0.35);
            color: #ffd5d5;
            background: rgba(255,107,107,0.10);
            padding: 5px 10px;
            border-radius: 999px;
            font-size: 12px;
            letter-spacing: .04em;
            text-transform: uppercase;
            font-weight: 700;
          }
          .title {
            margin: 12px 0 6px;
            font-size: 30px;
            line-height: 1.1;
            font-weight: 750;
            letter-spacing: -0.02em;
          }
          .subtitle {
            margin: 0;
            color: var(--muted);
            font-size: 14px;
            line-height: 1.5;
          }
          .grid {
            display: grid;
            grid-template-columns: 1.15fr 0.85fr;
            gap: 14px;
            padding: 16px;
          }
          .panel {
            background: var(--panel);
            border: 1px solid var(--border);
            border-radius: 14px;
            padding: 14px;
          }
          .panel h2 {
            margin: 0 0 10px;
            font-size: 13px;
            color: var(--muted);
            text-transform: uppercase;
            letter-spacing: .08em;
          }
          .callout {
            border: 1px solid rgba(255,184,107,0.25);
            background: rgba(255,184,107,0.07);
            color: #ffe0bb;
            border-radius: 12px;
            padding: 12px;
            font-size: 13px;
            line-height: 1.5;
          }
          .list {
            margin: 10px 0 0;
            padding: 0;
            list-style: none;
            display: grid;
            gap: 8px;
          }
          .list li {
            display: flex;
            gap: 10px;
            align-items: flex-start;
            color: var(--text);
            font-size: 13px;
            line-height: 1.45;
          }
          .dot {
            width: 8px;
            height: 8px;
            border-radius: 999px;
            margin-top: 5px;
            background: var(--accent);
            box-shadow: 0 0 0 4px rgba(79,209,197,0.14);
            flex: 0 0 auto;
          }
          .mono {
            font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
            font-size: 13px;
          }
          pre {
            margin: 0;
            white-space: pre-wrap;
            word-break: break-word;
            background: var(--panel-2);
            border: 1px solid var(--border);
            color: #eff7fa;
            border-radius: 12px;
            padding: 12px;
            font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
            font-size: 13px;
            line-height: 1.45;
          }
          .error {
            border-color: rgba(255,107,107,0.25);
            background:
              linear-gradient(180deg, rgba(255,107,107,0.07), rgba(255,107,107,0.03)),
              var(--panel-2);
          }
          .actions {
            display: flex;
            flex-wrap: wrap;
            gap: 10px;
            margin-top: 10px;
          }
          .btn {
            appearance: none;
            border: 1px solid var(--border);
            background: rgba(255,255,255,0.03);
            color: var(--text);
            border-radius: 10px;
            padding: 9px 12px;
            font-size: 13px;
            cursor: pointer;
            text-decoration: none;
          }
          .btn:hover { background: rgba(255,255,255,0.08); }
          .btn.primary {
            border-color: rgba(79,209,197,0.35);
            background: rgba(79,209,197,0.12);
          }
          .kv {
            display: grid;
            grid-template-columns: 88px 1fr;
            gap: 8px 10px;
            font-size: 13px;
            line-height: 1.4;
          }
          .kv dt { color: var(--muted); margin: 0; }
          .kv dd { margin: 0; }
          @media (max-width: 860px) {
            body { padding: 14px; }
            .grid { grid-template-columns: 1fr; padding: 12px; }
            .title { font-size: 24px; }
          }
        </style>
      </head>
      <body>
        <div class="shell">
          <div class="hero">
            <div class="badge">Startup Error · ${safeModeLabel}</div>
            <h1 class="title">Desktop shell couldn't connect</h1>
            <p class="subtitle">${safeModeHint}</p>
          </div>

          <div class="grid">
            <section class="panel">
              <h2>What To Do</h2>
              <div class="callout">${safeNextSteps}</div>
              <ul class="list">
                <li><span class="dot"></span><span>Frontend URL: <span class="mono">http://127.0.0.1:${webPort}</span></span></li>
                <li><span class="dot"></span><span>Backend health: <span class="mono">http://127.0.0.1:${apiPort}/api/v1/health</span></span></li>
                <li><span class="dot"></span><span>Mode: <span class="mono">${safeModeLabel}</span></span></li>
              </ul>
              <div class="actions">
                <button class="btn primary" onclick="location.reload()">Reload Window</button>
                <a class="btn" href="http://127.0.0.1:${webPort}">Open Frontend URL</a>
                <a class="btn" href="http://127.0.0.1:${apiPort}/api/v1/health">Open Health URL</a>
              </div>
            </section>

            <section class="panel">
              <h2>Run Commands</h2>
              <pre>${commandBlock}</pre>
              <div style="height:10px"></div>
              <h2>Environment</h2>
              <dl class="kv">
                <dt>API Port</dt><dd class="mono">${apiPort}</dd>
                <dt>Web Port</dt><dd class="mono">${webPort}</dd>
                <dt>Repo Root</dt><dd class="mono">${safeRepoRoot}</dd>
              </dl>
            </section>

            <section class="panel" style="grid-column: 1 / -1;">
              <h2>Raw Error</h2>
              <pre class="error">${errorText}</pre>
            </section>
          </div>
        </div>
      </body>
    </html>
  `;

  new BrowserWindow({
    title: `${shellTitle} (startup error)`,
    html,
    frame: { x: 80, y: 80, width: 960, height: 640 },
    titleBarStyle: "default",
    sandbox: true,
  });
}

async function main(): Promise<void> {
  Electrobun.events.on("before-quit", async () => {
    await shutdownChildren();
  });

  if ((startBackend || startFrontend) && !repoRoot) {
    throw new Error(
      "RARA_DESKTOP_REPO_ROOT is required when using managed server mode " +
      "(RARA_DESKTOP_START_BACKEND/FRONTEND).",
    );
  }

  if (startBackend) {
    spawnLogged("backend", ["just", "run"], repoRoot!);
    await waitForHttp(`http://127.0.0.1:${apiPort}/api/v1/health`, 180_000);
  }

  if (startFrontend) {
    spawnLogged("frontend", ["just", "web"], repoRoot!);
    await waitForHttp(`http://127.0.0.1:${webPort}`, 60_000);
  }

  const win = createAppWindow(`http://127.0.0.1:${webPort}`);

  if (process.env.RARA_DESKTOP_OPEN_DEVTOOLS === "1") {
    try {
      win.webview.openDevTools();
    } catch (error) {
      console.warn("[desktop] failed to open devtools", error);
    }
  }
}

void main().catch((error) => {
  console.error("[desktop] startup error", error);
  createErrorWindow(error);
});
