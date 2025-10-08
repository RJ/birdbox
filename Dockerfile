# Stage 1: Build
FROM rust:1.90-bookworm as builder

WORKDIR /usr/src/birdbox-rs

# Install build dependencies
RUN apt-get update && \
    apt-get install -y \
    clang \
    libclang-dev \
    llvm-dev \
    pkg-config \
    libopus-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY doorbird ./doorbird
COPY templates ./templates

# Build for release
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y libopus0 ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /usr/src/birdbox-rs/target/release/birdbox-rs .

# Copy templates
COPY templates ./templates

# Expose the HTTP/WebSocket port
EXPOSE 3000

# Expose single UDP port for WebRTC media (using UDP mux)
EXPOSE 50000/udp

# Run the server
CMD ["./birdbox-rs"]

