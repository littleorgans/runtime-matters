set shell := ["bash", "-cu"]

build:
    cargo build --workspace

test:
    cargo test --workspace

insta-test:
    cargo insta test --all

insta-accept:
    cargo insta test --all --accept

bench-spawn:
    cargo bench -p rtm-cli --bench spawn_latency

bench-status:
    cargo bench -p rtm-cli --bench status_query

load-test:
    cargo run --release -p rtm-cli --example load_test -- --sessions 50

dist-plan:
    dist plan

dist-build:
    dist build

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

check-loc:
    bash scripts/check-loc-limit.sh

check: fmt-check check-loc clippy
