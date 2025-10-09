# Stage 1: Install cargo-chef
FROM rust:1.90-bookworm as chef

WORKDIR /usr/src/birdbox-rs

# Install build dependencies
RUN apt-get update && \
    apt-get install -y \
    clang \
    libclang-dev \
    llvm-dev \
    pkg-config \
    nasm \
    libopus-dev \
    && rm -rf /var/lib/apt/lists/*

# Install cargo-chef
RUN cargo install cargo-chef

# Stage 2: Planner - Generate recipe for dependencies
FROM chef as planner

COPY Cargo.toml Cargo.lock ./
COPY doorbird ./doorbird
COPY src ./src
COPY templates ./templates

RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder - Build dependencies (this layer is cached)
FROM chef as builder

COPY --from=planner /usr/src/birdbox-rs/recipe.json recipe.json

# Build dependencies only - this layer is cached unless Cargo.toml/Cargo.lock changes
RUN cargo chef cook --release --recipe-path recipe.json

# Stage 4: Build application - Copy source and build (fast incremental build)
COPY src ./src
COPY doorbird ./doorbird
COPY templates ./templates
COPY Cargo.toml Cargo.lock ./

# Build the application (dependencies already built, so this is fast)
RUN cargo build --release

# Stage 5: Runtime
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

