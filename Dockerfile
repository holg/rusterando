FROM rust:latest AS builder

RUN rustup target add wasm32-unknown-unknown
RUN cargo install cargo-leptos

# Install tailwindcss CLI
RUN npm install -g tailwindcss@3 || true

WORKDIR /app
COPY . .
RUN cargo leptos build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY --from=builder /app/target/server/release/rusterando-server .
COPY --from=builder /app/target/server/release/hash.txt .
COPY --from=builder /app/target/site ./site

ENV LEPTOS_SITE_ROOT=/app/site
ENV LEPTOS_OUTPUT_NAME=rusterando
ENV LEPTOS_SITE_ADDR=0.0.0.0:3001
ENV LEPTOS_HASH_FILES=true
EXPOSE 3001

CMD ["./rusterando-server"]
