# =============================================================================
# Frontend Dockerfile — multi-stage build with nginx for serving
# =============================================================================

# ---------------------------------------------------------------------------
# Stage 1: builder — install deps and build the Vite/React app
# ---------------------------------------------------------------------------
FROM node:22-alpine AS builder
WORKDIR /app
COPY web/package.json web/package-lock.json* ./
RUN npm ci
COPY web/ .
RUN npm run build

# ---------------------------------------------------------------------------
# Stage 2: runtime — serve with nginx
# ---------------------------------------------------------------------------
FROM nginx:alpine AS runtime
COPY --from=builder /app/dist /usr/share/nginx/html
COPY docker/nginx.conf /etc/nginx/templates/default.conf.template
EXPOSE 80

# API_URL is substituted into nginx config at container start via envsubst
ENV API_URL=http://app:25555
