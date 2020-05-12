#! /usr/bin/bash

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && cd .. && pwd)"
PARAMS=$@

podman run --network=host --rm -it -v $SRC_DIR:/devel:z \
	-e RUST_BACKTRACE=1 btfm:dev \
	bash -c "cd /devel && cargo test && cargo clippy --all-targets --all-features -- -D warnings && cargo doc && cargo fmt -- --check -v"
