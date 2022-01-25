#!/bin/sh
# build statically linked binary with musl
# -> ./target/x86_64-unknown-linux-musl/release/pv_informant
podman run --rm -u root -v "$PWD":/app:Z -w /app clux/muslrust cargo build --release

