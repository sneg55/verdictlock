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
| Compound claims | Two axis words pulling opposite ways is a compound statement ("the image is authentic, the caption is false"), three or more spanning every option is an answer asserting everything at once. Only the second is a contradiction. |
| Adjacency | Every content word of the ground truth, none of its pairings: "France is the capital of Paris". |

An answer that contradicts nothing has passed every one of those tests, and at that
point the wording decides how far above the bar it sits, not whether it is right. What
it is graded on is salience-weighted recall of the part the question did not already
give away, precision, character trigrams and content-word adjacency. Figures and names
carry the weight, because they carry the answer: "Alibaba Cloud" is the answer to who
disclosed Log4Shell and "security team" is the sentence around it. Words match on their
stem, so `engine`/`engines`, `flag`/`flagged` and `splice`/`splicing` are the same word;
`US` is `United States` and `C` after a figure is `Celsius`; and `Thirty seconds` is
`30 seconds`.

An answer with too little in common with the ground truth to have been tested at all
does not qualify for that treatment, which is what keeps assistant boilerplate and a
repeated question near zero.

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
1.07 MB). `verify.sh` downloads it from the URL recorded in that registration and
checks its hash; it is not vendored here.

| corpus | cases | VerdictLock margin | champion margin | VerdictLock wins | champion wins |
| --- | ---: | ---: | ---: | ---: | ---: |
| `bench/url-scan.json` | 26 | **0.9018** | 0.4103 | **26/26** | 20/26 |
| `external/benchmark.json` (20 intents) | 40 | **0.9171** | 0.8475 | 40/40 | 40/40 |
| `external/family-numeric.json` | 15 | **0.8907** | 0.6860 | **15/15** | 14/15 |
| `external/family-authenticity.json` | 14 | **0.9063** | 0.4807 | 14/14 | 14/14 |
| `external/family-reference.json` | 12 | **0.8495** | 0.6447 | 11/12 | 11/12 |

Structural gates 15/15 and the gaming suite 18/18 on every corpus. `worst_self_match`
is 1.0 throughout, `score_stddev` 0.46 to 0.49. The champion scores 8/18 on the gaming
suite: it gives a swapped target 0.48, a prompt injection 0.31, and a correct terse
answer 0.00.

Every number above comes from `bench/report.json`, which is the last run of the
harness against the checked-in binary, so it can be diffed rather than trusted.

### Where it loses

One case across all 107: `ref-ip-hosting`, where the ground truth says AWS and the good
answer says Amazon. Both modules score it 0.000, so neither wins it. The champion's own
README records the same case as its known miss.

Word vectors were tried and dropped. The failing cases here are not synonym problems:
the answer is a name or a figure the ground truth also carries, and what was missing was
salience, not similarity. GloVe puts `increase` and `decrease` at 0.81 cosine, closer
than `rise` and `increase` at 0.67, so an embedding cannot tell a correct answer from
its opposite anyway. That is what the axes above are for.

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

Compiled size: 22 KB.

## Verify

```bash
./verify.sh
```

Builds, then runs every corpus against both binaries and exits non-zero if a gate fails
or a wrong answer outscores a right one. A tie is reported and does not fail the run:
`ref-ip-hosting` ties at 0.000 for both modules. What it checks, in the node's own terms:

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
sha256    34378be0d0ad0a9a21eb05da785ec38c4e6fe78f0fc3091f6d985ca5f644c0c5
keccak256 0x08245b3ecca741b81883c19e34ea2e6289b731af946ea32c31b7cef2bd751122
bytes     22193
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

## Licence

MIT. See `LICENSE`.
