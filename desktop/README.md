# Rara Desktop (Electrobun)

This folder adds a local desktop shell using [Electrobun](https://github.com/blackboardsh/electrobun).

Current scope (default mode):

- Uses a split frontend/backend setup (you run services separately)
- Opens an Electrobun desktop window at `http://127.0.0.1:5173`
- Does not bundle/embed the Rust backend binary into `desktop/`

## Usage

From repo root:

```bash
just run   # terminal 1
just web   # terminal 2
just desktop
```

Managed mode (optional, desktop shell starts both dev servers for you):

```bash
just desktop-managed
```

Or directly:

```bash
cd desktop
bun install
bun run start
```

## Optional env vars

- `RARA_DESKTOP_API_PORT` (default `25555`)
- `RARA_DESKTOP_WEB_PORT` (default `5173`)
- `RARA_DESKTOP_OPEN_DEVTOOLS=1` to auto-open webview devtools

## Notes

- This is a dev-shell integration (desktop wrapper around your existing local dev servers).
- Production packaging can be added later; current implementation keeps frontend/backend runtime separate.
- `electrobun@1.14.4` currently ships TS sources that trigger upstream `tsc` type errors in `node_modules` (not from this repo), so local validation is best done by running the app/build command directly.
