#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
task_root=$(mktemp -d "${TMPDIR:-/tmp}/ludomere-checks.XXXXXX")
trap 'rm -rf -- "$task_root"' EXIT
export XDG_CONFIG_HOME="$task_root/config"
export XDG_DATA_HOME="$task_root/data"
export XDG_CACHE_HOME="$task_root/cache"
export XDG_STATE_HOME="$task_root/state"
export XDG_RUNTIME_DIR="$task_root/runtime"
mkdir -m 700 -- "$XDG_RUNTIME_DIR"
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
# Fixtures exercise process-wide operation/account guards; separate tests must
# not contend for those guards. Individual concurrency tests still spawn threads.
cargo test --locked -- --test-threads=1
python3 resources/helpers/test_umu_boundary.py
