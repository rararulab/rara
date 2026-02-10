FROM oven/bun:1-alpine AS build
WORKDIR /app
COPY web/package.json web/bun.lock* ./
RUN bun install --frozen-lockfile 2>/dev/null || bun install
COPY web/ .
RUN bun run build

FROM nginx:alpine
COPY --from=build /app/dist /usr/share/nginx/html
COPY docker/nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
