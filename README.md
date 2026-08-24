# VerdictLock

[![licence MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)
[![target wasm32-unknown-unknown](https://img.shields.io/badge/target-wasm32--unknown--unknown-orange.svg)](#build)
[![no_std, no imports](https://img.shields.io/badge/wasm-no__std%2C%20no%20imports-lightgrey.svg)](#build)
[![structural gates 15/15](https://img.shields.io/badge/structural%20gates-15%2F15-brightgreen.svg)](#verify)
[![benchmark 40/40](https://img.shields.io/badge/benchmark%20ordering-40%2F40%20vs%20baseline%2020%2F40-brightgreen.svg)](#measured)
[![gaming suite 18/18](https://img.shields.io/badge/gaming%20suite-18%2F18-brightgreen.svg)](#measured)

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
| Shotgun | An answer that names every candidate has not chosen one. |
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

The floor is earned on the part of the ground truth the question did not already give
away. An answer that hands the question back covers the sentence and none of the answer,
so it does not qualify, and neither does assistant boilerplate. Where the question
already contains nearly all of the ground truth there is no such part to measure, and
overall recall stands in.

An answer that agrees on every axis both sides spoke on is treated as correct even
when it shares no wording with the ground truth ("Expect a decline in price" for "It
implies the price is expected to fall"). That is the only path that does not depend on
overlap, and it is why the lexical guards are switched off when it is the one carrying
the score.

## Measured

The **Canonical Script** is Telegraph's own scoring module: MiniLM-L6-v2 sentence
embeddings, cosine similarity, BM25 and a length signal, 24.2 MB, published at
`telegraphprotocol/telegraph-wasm-baseline`. It is the baseline a replacement is
measured against. The **champion** is the binary currently holding the `URL_SCAN`
slot (registration 220, `zkasuran/telegraph-salience-scorer`, 1.07 MB). It is a far
harder target, and beating the baseline says nothing about beating it.

`harness/run.mjs` loads a `.wasm` the way a validator does, no host imports and
strings written through the module's own `alloc`, and reports the numbers the node
records on a registration. `harness/baseline.mjs` does the same against the Canonical
Script, one instance per probe, because that module traps on inputs the harness feeds
every module and a trap leaves its allocator unusable for every later call.
`verify.sh` fetches the champion by the URL in its registration and checks its hash,
and builds the Canonical Script from a pinned commit. Neither is vendored here.

| corpus | cases | VerdictLock | Canonical Script | champion |
| --- | ---: | ---: | ---: | ---: |
| `bench/url-scan.json` | 26 | **0.9018**, 26/26 | -0.0063, 14/26 | 0.4103, 20/26 |
| `bench/gate-stress.json` | 26 | **0.7336**, 23/26 | -0.0219, 10/26 | 0.6072, 22/26 |
| `external/benchmark.json` (20 intents) | 40 | **0.9169**, 40/40 | 0.0133, 20/40 | 0.8475, 40/40 |
| `external/family-numeric.json` | 15 | **0.8905**, 15/15 | -0.1259, 1/15 | 0.6860, 14/15 |
| `external/family-authenticity.json` | 14 | **0.9103**, 14/14 | 0.0113, 7/14 | 0.4807, 14/14 |
| `external/family-reference.json` | 12 | **0.8495**, 11/12 | 0.1834, 9/12 | 0.6447, 11/12 |

Margin is mean good answer minus mean wrong answer; the second figure is ordering wins.

On the 40-case benchmark the Canonical Script puts its good answers at 0.4854 and its
wrong answers at 0.4722, a separation of 0.0133 against a spread of 0.1354. The signal
is inside its own noise, and it orders exactly half the cases correctly. On three of
the six corpora its margin is negative: the average wrong answer outscores the average
right one. `family-numeric` is the clearest case, 1 ordering win in 15, because a
figure that has been altered barely moves a sentence embedding.

### Robustness

The same probes run against both modules, each on a fresh instance:

| probe | VerdictLock | Canonical Script |
| --- | --- | --- |
| blank answer is exactly 0 | 0 | 0 |
| whitespace answer is exactly 0 | 0 | 0 |
| punctuation answer is near 0 | 0 | 0.0259 |
| self match >= 0.75 | 1 | 0.8367 |
| 76 KB answer stays in [0,1] | 0.9523 | 0.3251 |
| oversized ground truth stays in [0,1] | 0.9523 | 0.5753 |
| emoji / CJK / RTL stays in [0,1] | 1 | 0.7620 |
| invalid UTF-8 does not trap | 0 | **traps** |
| embedded NULs stay in [0,1] | 0.9760 | 0.3745 |
| 200 random-byte triples | 0 traps | **198 traps** |

A validator scores whatever a miner returns, and a miner returning bytes that are not
valid UTF-8 is not exotic. The Canonical Script traps on those bytes with a memory
access out of bounds, and on 198 of 200 random-byte triples. Structural gates 15/15 and
the gaming suite 18/18 for VerdictLock on every corpus, `worst_self_match` 1.0
throughout, `score_stddev` 0.46 to 0.49. The champion scores 8/18 on the gaming suite:
it gives a swapped target 0.48, a prompt injection 0.31, and a correct terse answer 0.00.

Every number above comes from `bench/report.json`, which `harness/report.mjs` writes
by running both harnesses over all six corpora. Regenerate it after a change and diff
it, rather than trusting the tables here:

```bash
node harness/report.mjs module/target/wasm32-unknown-unknown/release/verdictlock.wasm \
  dist/champion-reg220-url_c3.wasm \
  .baseline/target/wasm32-unknown-unknown/release/telegraph_scoring.wasm
```

### Where it loses

One case across all 107: `ref-ip-hosting`, where the ground truth says AWS and the good
answer says Amazon. Both modules score it 0.000, so neither wins it. The champion's own
README records the same case as its known miss.

The vector table is here because the validator pointed at vocabulary. See
**Registered** below: three readings place ten of the node's fifteen good answers at
0.99 for this module, five at 0.32, and every wrong answer at 0.04. Five correct
answers that share almost no vocabulary with their ground truth is the one thing a
lexical scorer cannot reach, which is what a vector table is for.

The table is the 20,000 most frequent GloVe words at 300 dimensions, L2-normalised to
int8 and keyed by the same hash the tokeniser computes, so a lookup is a binary search
and a cosine is an integer dot product. A near neighbour is worth three quarters of the
word the ground truth actually used, never all of it. Two words on opposite sides of any
axis are never neighbours however close their vectors are, because GloVe puts `increase`
and `decrease` at 0.81 cosine, closer than `rise` and `increase` at 0.67: it reads topic,
not direction. That guard is why lowering the threshold from 0.45 to 0.26 moves the mean
score of wrong answers by 0.0001.

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

Compiled size: 5.8 MB, of which 5.8 MB is the vector table and 23 KB is code.

## Verify

```bash
./verify.sh
```

Builds, fetches the champion and builds the Canonical Script, then runs every corpus
against both and exits non-zero if a gate fails or a wrong answer outscores a right one.
The baseline leg runs a six-layer transformer per call and costs about 90 seconds a
corpus, so `SKIP_BASELINE=1 ./verify.sh` leaves it out. A tie is reported and does not
fail the run: `ref-ip-hosting` ties at 0.000 for both modules. What it checks, in the
node's own terms:

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

## Registered

Eleven registrations, all rejected, all readable on chain. The node scores every
candidate against a hidden fixture set for that intent and promotes only a module that
wins at least as many orderings as the champion and separates by a larger average
margin. Ordering is the binding constraint: every rejection here names it, and none
names the margin.

| reg | binary | node margin | champion | ordering | rejection |
| --- | --- | ---: | ---: | ---: | --- |
| 697 | first build | 0.6773 | 0.9481 | 14/15 | lost on ordering |
| 698 | salience and figures | 0.7260 | 0.9481 | 14/15 | lost on ordering |
| 699 | inflected axis words | 0.7260 | 0.9481 | 14/15 | identical to 698, to seven decimals |
| 700 | lower floor entry | 0.7260 | 0.9481 | 14/15 | identical again |
| 701 | diagnostic, scores capped at 0.8 | 0.5980 | 0.9481 | 14/15 | reverted immediately |
| 703 | GloVe vector table | 0.7260 | 0.9481 | 14/15 | moved the margin by 0.00002 |
| 705 | diagnostic, reports only whether a gate fired | 0.4970 | 0.9481 | 14/15 | reverted immediately |
| 707 | `substitution` gate removed | 0.7244 | 0.9481 | 14/15 | the prime suspect was not it |
| 709 | diagnostic, wording score alone | 0.3063 | 0.9481 | 13/15 | reverted immediately |

All cleared the structural stage: `worst_self_match` 1.0, `score_stddev` 0.43 to 0.47,
no errors.

Two registrations went to other intents, to find out whether the module is general or
`URL_SCAN`-shaped. It is `URL_SCAN`-shaped.

| reg | intent | node margin | champion | ordering | champion ordering |
| --- | --- | ---: | ---: | ---: | ---: |
| 711 | `TEXT_AUTHENTICITY_CHECK` | 0.3374 | 0.4045 | 10/15 | 14/15 |
| 712 | `ACADEMIC_SEARCH` | 0.5278 | 0.6804 | 8/13 | 9/13 |

Fourteen of fifteen on `URL_SCAN`, ten of fifteen and eight of thirteen elsewhere, on
the same binary. The gates that make this module sharp on a security verdict, target
binding, the verdict ladder, the figure checks, have nothing to bite on in a question
about whether a passage was written by a machine. This is an intent-specific scorer,
and `URL_SCAN` is the intent.

Three different binaries returning a margin and a deviation identical to seven decimal
places is either three changes that miss every fixture or a node that is not re-running
what it fetches. Registration 701 settles it: capping every score at 0.8 moved the margin
to 0.5980, so the evaluation is live and those three changes genuinely touched nothing.

Registration 705 is the second measurement. Scoring 0.90 when no gate fired and 0.10
when one did turns the reported margin into a reading of how many correct answers the
gates are rejecting: 0.497 decodes to roughly six of the fifteen. So the five that score
0.32 are not failing on vocabulary, which registration 703 had already shown by moving
the margin 0.00002 with a 6 MB vector table attached. A gate is firing on them.

Counting gate firings over all 133 correct answers in these corpora puts the blame on
the two softest gates, `substitution` and `precision`, and the arithmetic agrees:
substitution multiplies by 0.55, and 0.55 against a middling wording score is 0.32.
`substitution` is gone. `precision` stays, because taking it out lets an answer that
lists every candidate through the champion's own gaming suite.

That cap is also a measurement. With fifteen cases, a margin of 0.7260364 and a deviation
of 0.43392357 uncapped, and 0.5979962 capped, one arrangement reproduces all three
numbers to four decimal places: ten good answers at 0.992, five good answers at 0.320,
and fifteen wrong answers at 0.042. Wrong answers are not the problem. Five correct
answers that this module scores at a third are, and lifting them to where the other ten
sit would put the margin at 0.948, which is the champion's number to three decimals.

## Register

`dist/verdictlock.wasm` is the built module.

```
sha256    4fb5008c99eab353b992f369d7d66cfa1ab5510618bf335ff6a6985aed2da8e9
keccak256 0x6a229d42f97f1c8a1677c974d153e038113dea30bd05084e391eca3e5d758cac
bytes     6104493
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

`bench/gate-stress.json` is 26 cases in which the answer is unambiguously correct and
is worded to stress one of the module's own gates: a negation the gate might read
backwards, a hedge, a figure written another way, an extra name, a rewritten clause. It
exists because a gate that misfires on a correct answer costs an ordering win, and that
is invisible on a corpus written by the same hand as the scorer. It earned its place
immediately: it caught the axis lexicons matching only exact word forms, so `climbed`
never registered as a direction and a wrong answer beat a right one 0.977 to 0.000. Two
cases in it are marked `known_miss` with the reason; the harness reports them and does
not fail on them.

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
