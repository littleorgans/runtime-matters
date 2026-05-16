set shell := ["bash", "-cu"]

build:
    cargo build --workspace

test:
    cargo test --workspace

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

check-loc:
    bash scripts/check-loc-limit.sh

check: fmt-check check-loc clippy

