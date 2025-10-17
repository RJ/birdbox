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

# Stage 5: Extract runtime libraries from Debian
FROM debian:bookworm-slim as runtime-deps

RUN apt-get update && \
    apt-get install -y libopus0 libssl3 && \
    rm -rf /var/lib/apt/lists/*

# own the libs (from arch-specific paths)
# (no {} brace expansion in this shell).
RUN mkdir /mylibs && cd / && tar vcf - /usr/lib/*-linux-gnu/libopus.so.* /usr/lib/*-linux-gnu/libssl.so.* /usr/lib/*-linux-gnu/libcrypto.so.* | tar xf - -C /mylibs
RUN find /mylibs/

# Stage 6: Final runtime with distroless
FROM gcr.io/distroless/cc-debian12

WORKDIR /app

# Copy and unpack our libs, preserving the arch-specific paths
COPY --from=runtime-deps /mylibs/* /

# Copy the binary from builder
COPY --from=builder /usr/src/birdbox-rs/target/release/birdbox-rs .

COPY templates ./templates

# Expose the HTTP/WebSocket port
EXPOSE 3000

# Expose single UDP port for WebRTC media (using UDP mux)
EXPOSE 50000/udp

# Run the server
CMD ["./birdbox-rs"]
