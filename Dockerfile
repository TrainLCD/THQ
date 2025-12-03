FROM rust:1.91-slim AS builder
WORKDIR /app

# Cache dependency build
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src

# Build actual binary
COPY src ./src
RUN cargo build --release

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/thq-server /usr/local/bin/thq-server

EXPOSE 8080
CMD ["thq-server", "--host", "0.0.0.0", "--port", "8080"]
