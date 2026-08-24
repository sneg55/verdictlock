# VerdictLock

A Telegraph scoring module for `URL_SCAN`: the WASM program a Telegraph validator
runs to decide how good a miner's answer was. It takes the question, the ground
truth and the miner's answer and returns one `f32` between 0 and 1.

The module the protocol ships with scores word overlap. On a URL scan that is the
wrong measurement, because the two answers that matter most are almost the same
string:

```
ground truth   https://example.test/login is malicious; 12 engines flagged it. Block access.
miner answer   https://example.test/login is benign;    12 engines flagged it. Allow access.
```

One word decides whether an agent opens the link. VerdictLock reads the facts that
flip an answer before it lets wording count for anything: which target the answer is
about, which verdict it reaches, whether the record exists, which way a figure moved,
and what the figures actually are.

## What it checks before overlap

| Check | What it catches |
| --- | --- |
| Target binding | The right verdict about a URL nobody asked about. `paypa1-security.com` for `paypal-security.com`, `/login/signup` for `/login/reset`, a different hash of the same length. |
| Verdict axis | malicious / suspicious / clean, negation aware, read on both sides. `0 malicious` and `not malicious` and `"malicious": false` are all denials. |
| No-verdict axis | A scan that timed out, was rate limited or is still processing did not return a clean bill of health. Read separately, because "unknown" contradicts "safe" as much as it contradicts "malicious". |
| Record axis | `in PhishTank, verified` against `in PhishTank, not yet verified`. Suppressed whenever the answer hedges, so a pending scan is never scored as a contradiction. |
| Direction axis | `strengthened` against `weakened`, `rise` against `fall`. |
| Yes/no and authenticity | Read as two axes, not one, because "No, written by a human" is negative on the first and positive on the second at the same time. |
| Figures | Every figure in the ground truth, compared without thousands separators, with its scale word (`3.1 trillion` is `$3.1T` and is not `3.1 billion`) and against the field it is attached to (`26 malicious, 41 harmless` is not `41 malicious, 26 harmless`). |
| Strangers | A capitalised name the question and ground truth never mention: "a Cloudflare edge address" where the truth says "a Tor exit node". |
| Adjacency | Every content word of the ground truth, none of its pairings: "France is the capital of Paris". |

What survives all of that is graded on wording: salience-weighted recall of the part
the question did not already give away, precision, character trigrams and content-word
adjacency. Words match on their stem, so `engine`/`engines`, `flag`/`flagged` and
`splice`/`splicing` are the same word, and `US` is `United States`.

An answer that agrees on every axis both sides spoke on is treated as correct even
when it shares no wording with the ground truth ("Expect a decline in price" for "It
implies the price is expected to fall"). That is the only path that does not depend on
overlap, and it is why the lexical guards are switched off when it is the one carrying
the score.

## Measured

`harness/run.mjs` loads a `.wasm` the way a validator does (no host imports, strings
written through the module's own `alloc`) and reports the numbers the node records on
a registration. Every corpus is run against the binary that currently holds the
`URL_SCAN` champion slot (registration 220, `zkasuran/telegraph-salience-scorer`,
1.07 MB), pulled from the public registry.

| corpus | cases | VerdictLock margin | champion margin | VerdictLock wins | champion wins |
| --- | ---: | ---: | ---: | ---: | ---: |
| `bench/url-scan.json` | 26 | **0.8823** | 0.4103 | **26/26** | 20/26 |
| `external/benchmark.json` (20 intents) | 40 | **0.8798** | 0.8475 | 40/40 | 40/40 |
| `external/family-numeric.json` | 15 | **0.8316** | 0.6860 | **15/15** | 14/15 |
| `external/family-authenticity.json` | 14 | **0.7897** | 0.4807 | 14/14 | 14/14 |
| `external/family-reference.json` | 12 | 0.5790 | **0.6447** | 10/12 | **11/12** |

Structural gates 15/15 and the gaming suite 18/18 on every corpus. `worst_self_match`
is 1.0 throughout, `score_stddev` 0.46 to 0.47. The champion scores 8/18 on the gaming
suite: it gives a swapped target 0.48, a prompt injection 0.31, and a correct terse
answer 0.00.

Every number above comes from `bench/report.json`, which is the last run of the
harness against the checked-in binary, so it can be diffed rather than trusted.

### Where it loses

`family-reference.json` is the one corpus the champion wins, on two cases:
`ref-ip-hosting` (ground truth says AWS, the good answer says Amazon) and
`ref-news-jwst` (ground truth says 25 December, the good answer says Christmas Day).
Both need an alias table this module does not ship. The champion's own README records
the first of those as its known miss too; it wins them by scoring on embedded vectors,
which is what the other megabyte of that binary is.

## Build

```bash
rustup target add wasm32-unknown-unknown        # once
cd module && cargo build --release --target wasm32-unknown-unknown
```

Must be `wasm32-unknown-unknown`. A `wasm32-wasip1` build carries WASI imports and a
validator runs modules with nothing bound, so it fails to instantiate.

`no_std`, no allocator beyond a bump pointer into a static heap, no imports at all.
Every buffer is a fixed static and every loop is bounded, so a 76 KB answer costs a
predictable amount of work. All parsing is byte level: the input is whatever a miner
sent, so emoji, CJK, right-to-left script and invalid UTF-8 all have to score without
trapping.

Compiled size: 20 KB.

## Verify

```bash
./verify.sh
```

Builds, then runs every corpus against both binaries and exits non-zero if a gate
fails. What it checks, in the node's own terms:

- loads with no imports and exports `alloc`, `dealloc`, `rank_answer` and memory
- a blank, whitespace or punctuation-only answer scores exactly 0
- a perfect answer scores at least 0.75, and beats an unrelated one on every case
- a 76 KB answer, an oversized ground truth, emoji, CJK, RTL, invalid UTF-8, embedded
  NULs and 200 random-byte triples all stay inside [0,1] without trapping
- the same call returns the same score after other calls have run in between
- then the benchmark and the gaming suite

Two extras:

```bash
BENCH=bench/external/benchmark.json ./harness/run.mjs …   # a different corpus
node harness/probe.mjs <wasm> <case-id>                   # the parts behind one score
```

`probe` needs `cargo build --release --target wasm32-unknown-unknown --features probe`.
The registered binary is built without it and exports nothing but the ABI.

## Register

`dist/verdictlock.wasm` is the built module.

```
sha256    ec2d43f03dfa599160b74bc7fae2848995b672ffdce4759be2204ff13c688b52
keccak256 0x1903889e3b1f366a7cea14429b1b6a3551076b8391990c4cef1bfc8af7c5b3ba
bytes     20512
```

`node harness/keccak.mjs dist/verdictlock.wasm` reproduces the keccak hash (it
self-tests against the known digest of the empty string first).

Host the exact bytes at a public URL, then either submit at
[integrate.telegraphprotocol.com](https://integrate.telegraphprotocol.com), which
hashes the file and sends the transaction, or call the Diamond directly:

```solidity
registerWasm(bytes32 wasmHash, string wasmUrl, string intent)   // intent: "URL_SCAN"
```

Registration costs gas and nothing else: no bond, no fee. The node re-downloads the
file and re-hashes it, so a host that re-encodes on upload breaks the match. After
registration the node runs the structural checks, then scores the module against the
current champion on its own benchmark; the result and any rejection reason are
readable from the registry.

## Corpora

`bench/url-scan.json` is 26 cases written for this intent: a question, the ground
truth a validator holds, a correct answer worded differently, and the kind of wrong
answer a weak miner actually returns. Answer shapes follow the five miners live on
`URL_SCAN` (url-sentinel, virustotal, phishtank, urlscan, chainsight-oracle).
`bench/attacks.json` is 18 gaming and robustness cases. Neither is copied from
Telegraph's benchmark, which is not public.

`bench/external/` is four corpora from the champion's own repository, MIT licensed,
vendored so this module is measured against fixtures its author did not write. See
`bench/external/NOTICE.md`.
