#!/bin/bash
set -e

echo "Installing protoc..."
sudo apt-get update
sudo apt-get install -y protobuf-compiler

echo "Installing just..."
curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | sudo bash -s -- --to /usr/local/bin

echo "Installing Rust components..."
rustup component add clippy rustfmt

echo "Installing cargo tools..."
cargo install cargo-watch cargo-nextest

echo "Building project to warm up cache..."
cargo build

echo "Done! Development environment is ready."
