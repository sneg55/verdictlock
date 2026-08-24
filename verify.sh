#!/usr/bin/env bash
# Builds the module and measures it against every corpus, next to two references:
# the Canonical Script, which is the baseline Track 2 is judged against, and the
# binary that currently holds the URL_SCAN champion slot. Exits non-zero on a gate
# failure or a real ordering loss.
set -euo pipefail
cd "$(dirname "$0")"

(cd module && cargo build --release --target wasm32-unknown-unknown)
WASM=module/target/wasm32-unknown-unknown/release/verdictlock.wasm

# the binary holding the URL_SCAN champion slot, as recorded in registration 220
CHAMPION=dist/champion-reg220-url_c3.wasm
CHAMPION_URL=https://raw.githubusercontent.com/zkasuran/telegraph-salience-scorer/0174a85639c398a0e898dcb11b54367eb2723b2b/dist/xfmr/url_c3.wasm
CHAMPION_SHA=ee85db4661b262a6133f71f0b8f228e663d213cefdaf73c43c293c082bb00d0b
if [ ! -f "$CHAMPION" ]; then
  echo "fetching the champion binary from the registry URL"
  curl -sL --fail "$CHAMPION_URL" -o "$CHAMPION"
fi
echo "$CHAMPION_SHA  $CHAMPION" | shasum -a 256 -c -

# the Canonical Script: Telegraph's own MiniLM-L6-v2 + BM25 baseline, built from
# source at a pinned commit because the weights make it too large to vendor
BASELINE_REPO=https://github.com/telegraphprotocol/telegraph-wasm-baseline.git
BASELINE_REF=dfa0cf7fda72789267811ba2190f61a8eaacedf6
BASELINE_DIR=${BASELINE_DIR:-.baseline}
BASELINE=$BASELINE_DIR/target/wasm32-unknown-unknown/release/telegraph_scoring.wasm
if [ ! -f "$BASELINE" ]; then
  echo "building the Canonical Script from $BASELINE_REF"
  [ -d "$BASELINE_DIR" ] || git clone -q "$BASELINE_REPO" "$BASELINE_DIR"
  (cd "$BASELINE_DIR" && git checkout -q "$BASELINE_REF" &&
   cargo build --release --target wasm32-unknown-unknown --features real_weights 2>/dev/null)
fi

fail=0
for bench in bench/url-scan.json bench/gate-stress.json bench/external/benchmark.json bench/external/family-*.json; do
  [ "$(basename "$bench")" = "NOTICE.md" ] && continue
  echo
  echo "================ $(basename "$bench")"
  BENCH="$bench" node harness/run.mjs "$WASM" "$CHAMPION" || fail=1
  # the Canonical Script runs a 6-layer transformer per call, so this leg costs
  # about 90 seconds a corpus; SKIP_BASELINE=1 leaves it out
  [ "${SKIP_BASELINE:-0}" = "1" ] ||
    BENCH="$bench" node harness/baseline.mjs "$WASM" "$BASELINE" || fail=1
  # the champion's own gaming suite as well as ours
  ATTACKS=bench/external/attacks.json BENCH="$bench" node harness/run.mjs "$WASM" >/dev/null || fail=1
done
exit $fail
