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

# test:
#     just test-unit
#     just test-integration
test *ARGS:
    cargo nextest run --workspace {{ARGS}}

test-unit:
    cargo test --workspace --lib --bins
    cargo test --workspace --doc

test-integration:
    @set -eu; \
    targets="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; data=json.load(sys.stdin); names=[(p["name"], t["name"]) for p in data["packages"] for t in p["targets"] if "test" in t["kind"]]; print(chr(10).join(f"{p} {t}" for p,t in sorted(names)))')"; \
    if [ -z "$targets" ]; then \
        echo "No integration test targets"; \
        exit 0; \
    fi; \
    printf '%s\n' "$targets" | while read -r package target; do \
        echo "=== $package --test $target ==="; \
        timeout 120 cargo test -p "$package" --test "$target" -- --nocapture; \
    done

test-integration-one package target:
    timeout 120 cargo test -p "{{package}}" --test "{{target}}" -- --nocapture

linux-target-check:
    cargo check -p rtm-platform --target x86_64-unknown-linux-gnu
    cargo check -p lilo-rm-client --target x86_64-unknown-linux-gnu

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
    cargo clippy --workspace --fix --allow-dirty -- -D warnings

check-loc:
    bash scripts/check-loc-limit.sh

check: fmt clippy-fix check-loc
