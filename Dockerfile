FROM node:22-bookworm-slim AS frontend

WORKDIR /app/doudou_blend
COPY doudou_blend/package.json doudou_blend/package-lock.json ./
RUN npm ci
COPY doudou_blend/ ./
RUN npm run build

FROM rust:1.88-bookworm AS server-builder

WORKDIR /app
COPY blend_kit_rs/ ./blend_kit_rs/
COPY blend_kit_server/ ./blend_kit_server/
WORKDIR /app/blend_kit_server
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN groupadd --system app \
    && useradd --system --gid app --home-dir /app app
WORKDIR /app
COPY --from=server-builder /app/blend_kit_server/target/release/blend_kit_server /usr/local/bin/blend_kit_server
COPY --from=frontend /app/doudou_blend/dist/ /app/public/

ENV PORT=3000
ENV STATIC_DIR=/app/public
ENV RUST_LOG=blend_kit_server=info,tower_http=info
EXPOSE 3000

USER app
CMD ["blend_kit_server"]
