# syntax=docker/dockerfile:1

FROM oven/bun:1.3.9 AS deps

ENV NEXT_TELEMETRY_DISABLED=1
WORKDIR /app
COPY package.json bun.lock tsconfig.base.json ./
COPY npm-packages/dashboard/package.json npm-packages/dashboard/package.json
COPY npm-packages/sdk/package.json npm-packages/sdk/package.json
COPY packages/protocol/package.json packages/protocol/package.json
COPY packages/console/package.json packages/console/package.json
RUN bun install --frozen-lockfile

FROM deps AS build
COPY packages/protocol packages/protocol
COPY packages/console packages/console
COPY npm-packages/dashboard npm-packages/dashboard
RUN bun run build:packages
RUN bun run --cwd npm-packages/dashboard build

FROM node:22-bookworm-slim AS runtime

ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1
ENV HOSTNAME=0.0.0.0
ENV PORT=6791

RUN useradd --create-home --uid 10001 vifu
USER vifu
WORKDIR /app
COPY --from=build --chown=vifu:vifu /app/npm-packages/dashboard/.next/standalone ./
COPY --from=build --chown=vifu:vifu /app/npm-packages/dashboard/.next/static ./npm-packages/dashboard/.next/static
COPY --from=build --chown=vifu:vifu /app/npm-packages/dashboard/public ./npm-packages/dashboard/public

WORKDIR /app/npm-packages/dashboard
EXPOSE 6791
CMD ["node", "server.js"]
