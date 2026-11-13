#!/bin/sh
# Builds a statically linked CLI for release downloads.
# Needs the musl Rust target and a musl C compiler (Arch: rust-musl and musl).
set -eu
cd "$(dirname "$0")/.."
target="$(uname -m)-unknown-linux-musl"
cargo build --release --locked --target "$target" -p appimage-integrator
binary="target/$target/release/appimage-integrator"
file "$binary"
ls -l "$binary"
