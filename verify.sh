#!/usr/bin/env bash
# Builds the module and measures it against every corpus, next to the binary that
# currently holds the URL_SCAN champion slot. Exits non-zero if a gate fails.
set -euo pipefail
cd "$(dirname "$0")"

(cd module && cargo build --release --target wasm32-unknown-unknown)
WASM=module/target/wasm32-unknown-unknown/release/verdictlock.wasm
CHAMPION=dist/champion-reg220-url_c3.wasm

fail=0
for bench in bench/url-scan.json bench/external/*.json; do
  [ "$(basename "$bench")" = "NOTICE.md" ] && continue
  echo
  echo "================ $(basename "$bench")"
  BENCH="$bench" node harness/run.mjs "$WASM" "$CHAMPION" || fail=1
done
exit $fail
