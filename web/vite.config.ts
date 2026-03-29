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

import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import fs from 'node:fs'
import path from 'path'

function contentTypeFor(filePath: string): string {
  const ext = path.extname(filePath).toLowerCase()
  switch (ext) {
    case '.html': return 'text/html; charset=utf-8'
    case '.css': return 'text/css; charset=utf-8'
    case '.js': return 'application/javascript; charset=utf-8'
    case '.json': return 'application/json; charset=utf-8'
    case '.svg': return 'image/svg+xml'
    case '.png': return 'image/png'
    case '.jpg':
    case '.jpeg': return 'image/jpeg'
    case '.gif': return 'image/gif'
    case '.woff': return 'font/woff'
    case '.woff2': return 'font/woff2'
    default: return 'application/octet-stream'
  }
}

function docsBookPlugin() {
  const bookDir = path.resolve(__dirname, '../docs/book')

  return {
    name: 'docs-book-static',
    configureServer(server: import('vite').ViteDevServer) {
      server.middlewares.use((req, res, next) => {
        const rawUrl = req.url ?? '/'
        const pathname = rawUrl.split('?')[0]
        if (!pathname.startsWith('/book')) return next()

        if (!fs.existsSync(bookDir)) return next()

        const rel = pathname.replace(/^\/book/, '') || '/'
        const safeRel = rel.replace(/\\/g, '/')
        const candidate = path.resolve(bookDir, `.${safeRel}`)
        if (!candidate.startsWith(bookDir)) {
          res.statusCode = 400
          res.end('Bad Request')
          return
        }

        let filePath = candidate
        if (fs.existsSync(filePath) && fs.statSync(filePath).isDirectory()) {
          filePath = path.join(filePath, 'index.html')
        } else if (!path.extname(filePath)) {
          const htmlPath = `${filePath}.html`
          if (fs.existsSync(htmlPath)) filePath = htmlPath
        }

        if (!fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) return next()

        res.setHeader('Content-Type', contentTypeFor(filePath))
        fs.createReadStream(filePath).pipe(res)
      })
    },
    writeBundle() {
      if (!fs.existsSync(bookDir)) return
      const outDir = path.resolve(__dirname, 'dist', 'book')
      fs.mkdirSync(outDir, { recursive: true })
      fs.cpSync(bookDir, outDir, { recursive: true })
    },
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss(), docsBookPlugin()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    host: true,
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': {
        target: process.env.VITE_API_URL || 'http://localhost:25555',
        changeOrigin: true,
        ws: true,
        configure: (proxy) => {
          proxy.on('proxyRes', (proxyRes, req) => {
            if (req.url?.includes('/health')) {
              const status = proxyRes.statusCode ?? 0;
              const tag = status >= 200 && status < 300 ? '✓' : '✗';
              console.log(`[heartbeat] ${tag} ${req.method} ${req.url} → ${status}`);
            }
          });
          proxy.on('error', (err, req) => {
            if (req.url?.includes('/health')) {
              console.log(`[heartbeat] ✗ ${req.method} ${req.url} → ${err.message}`);
            }
          });
        },
      },
    },
  },
  preview: {
    host: true,
    port: 4173,
    strictPort: true,
  },
})
