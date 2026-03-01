FROM rust:1.93-trixie AS builder

WORKDIR /build

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/reflet-core/Cargo.toml crates/reflet-core/Cargo.toml
COPY crates/reflet-bgp/Cargo.toml crates/reflet-bgp/Cargo.toml
COPY crates/reflet-api/Cargo.toml crates/reflet-api/Cargo.toml
COPY crates/reflet/Cargo.toml crates/reflet/Cargo.toml

# Create stub source files so cargo can resolve the dependency graph
RUN mkdir -p crates/reflet-core/src crates/reflet-bgp/src crates/reflet-api/src crates/reflet/src && \
    echo "fn main() {}" > crates/reflet/src/main.rs && \
    touch crates/reflet-core/src/lib.rs crates/reflet-bgp/src/lib.rs crates/reflet-api/src/lib.rs

# Pre-build dependencies (this layer is cached unless Cargo.toml/lock change)
RUN cargo build --release --bin reflet 2>/dev/null || true

# Copy actual source code
COPY crates/ crates/

# Touch source files so cargo knows they changed after the stub build
RUN touch crates/reflet-core/src/lib.rs crates/reflet-bgp/src/lib.rs \
    crates/reflet-api/src/lib.rs crates/reflet/src/main.rs

# Build the real binary
RUN cargo build --release --bin reflet

FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create config directory
RUN mkdir -p /etc/reflet

# Copy binary from builder
COPY --from=builder /build/target/release/reflet /usr/local/bin/reflet

# Copy example config as reference
COPY config.toml.example /etc/reflet/config.toml.example

# HTTP API
EXPOSE 8080
# BGP
EXPOSE 179

ENTRYPOINT ["reflet", "--config", "/etc/reflet/config.toml"]
