FROM rust:1.91-slim AS builder
WORKDIR /app

# Fetch dependencies (caches target deps)
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs && cargo fetch && rm -rf src

# Build actual binary
COPY src ./src
RUN cargo build --release

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/thq-server /usr/local/bin/thq-server
RUN mkdir -p /app/static
COPY --from=builder /app/src/static/join.csv /app/static/join.csv

EXPOSE 8080
CMD ["thq-server", "--host", "0.0.0.0", "--port", "8080"]
