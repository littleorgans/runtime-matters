set shell := ["bash", "-cu"]

# Fall back to $HOME/.cargo/bin/rtm if RTM_LOCAL_BIN is not set in the host environment
RTM_LOCAL_BIN := env("RTM_LOCAL_BIN", env("HOME") / ".cargo/bin/rtm")

install: install-release

build:
    cargo build --workspace

release-build:
    cargo build --workspace --release

build-local:
    RTM_VERSION_INCLUDE_GIT_SHA=1 cargo build -p rtm-cli --bin rtm --profile install-local

build-install-release:
    RTM_VERSION_INCLUDE_GIT_SHA=0 cargo build -p rtm-cli --bin rtm --release

install-local: build-local
    @just _install-bin target/install-local/rtm

install-release: build-install-release
    @just _install-bin target/release/rtm

_install-bin src:
    @set -eu; \
    src="$(pwd)/{{src}}"; \
    dest="{{RTM_LOCAL_BIN}}"; \
    case "$dest" in /*) ;; *) dest="$(pwd)/$dest";; esac; \
    if [ "$src" = "$dest" ]; then \
        echo "Built $src"; \
    else \
        mkdir -p "$(dirname "$dest")"; \
        install -m 755 "$src" "$dest"; \
        echo "Installed $dest"; \
    fi; \
    "$dest" --version

test:
    cargo test --workspace

publish-dry-run:
    cargo publish -p lilo-rm-core -p lilo-rm-client --dry-run --allow-dirty

insta-test:
    cargo insta test --all

insta-accept:
    cargo insta test --all --accept

bench-spawn:
    cargo bench -p rtm-cli --bench spawn_latency

bench-status:
    cargo bench -p rtm-cli --bench status_query

load-test:
    cargo run --release -p rtm-cli --example load_test -- --target headless --sessions 50

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

clippy-fix:
    cargo clippy --fix --workspace --all-targets --allow-dirty --allow-staged -- -D warnings

check-loc:
    bash scripts/check-loc-limit.sh

check: fmt clippy-fix fmt-check check-loc clippy
