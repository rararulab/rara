FROM ubuntu:25.04
ENV LANG=en_US.utf8
WORKDIR /app
SHELL ["/bin/bash", "-c"]

# System dependencies
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    software-properties-common \
    libssl-dev \
    tzdata \
    curl \
    unzip \
    ca-certificates \
    git \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain
ARG RUST_TOOLCHAIN=stable
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --no-modify-path --default-toolchain none -y
ENV PATH=/root/.cargo/bin:$PATH
RUN rustup toolchain install ${RUST_TOOLCHAIN}

# Cargo tools
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
RUN cargo binstall cargo-nextest just --no-confirm
RUN cargo install sccache cargo-chef
