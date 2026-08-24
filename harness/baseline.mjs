// Measures a module against the Canonical Script, Telegraph's own MiniLM-L6-v2 +
// BM25 baseline and the reference Track 2 is judged against.
//
// It exists separately from run.mjs because the Canonical Script traps on inputs
// run.mjs feeds every module: a trap corrupts its allocator, so each probe gets a
// fresh instance and a trap is recorded rather than propagated.
import { readFile } from "node:fs/promises";

const REQUIRED_EXPORTS = ["alloc", "dealloc", "rank_answer", "memory"];
const FUZZ_CASES = 200;

async function instantiate(module) {
  const instance = await WebAssembly.instantiate(module, {});
  const { alloc, dealloc, rank_answer: rank, memory } = instance.exports;
  const put = (value) => {
    const b = value instanceof Uint8Array ? value : new TextEncoder().encode(value);
    const p = alloc(b.length);
    new Uint8Array(memory.buffer, p, b.length).set(b);
    return [p, b.length];
  };
  return (question, groundTruth, answer) => {
    const [qp, ql] = put(question);
    const [gp, gl] = put(groundTruth);
    const [ap, al] = put(answer);
    const value = rank(qp, ql, gp, gl, ap, al);
    dealloc(ap, al);
    dealloc(gp, gl);
    dealloc(qp, ql);
    return value;
  };
}

// Compiling a module with real transformer weights costs far more than
// instantiating it, so it is compiled once and re-instantiated per probe.
async function load(path) {
  const bytes = await readFile(path);
  const module = new WebAssembly.Module(bytes);
  const exported = WebAssembly.Module.exports(module).map((e) => e.name);
  return {
    name: path.split("/").pop(),
    bytes: bytes.length,
    missing: REQUIRED_EXPORTS.filter((e) => !exported.includes(e)),
    score: await instantiate(module),
    fresh: () => instantiate(module),
  };
}

const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);
const stddev = (xs) => {
  if (xs.length < 2) return 0;
  const m = mean(xs);
  return Math.sqrt(xs.reduce((a, x) => a + (x - m) ** 2, 0) / (xs.length - 1));
};
const r4 = (x) => Number(x.toFixed(4));
const pad = (s, n) => String(s).padEnd(n);

function ordering(scorer, cases) {
  const goods = [];
  const bads = [];
  const selfs = [];
  let wins = 0;
  let ties = 0;
  let traps = 0;
  for (const c of cases) {
    const gt = c.ground_truth ?? c.groundTruth;
    let good = NaN;
    let bad = NaN;
    try { good = scorer.score(c.question, gt, c.good ?? c.good_answer); } catch { traps += 1; }
    try { bad = scorer.score(c.question, gt, c.bad ?? c.bad_answer); } catch { traps += 1; }
    try { selfs.push(scorer.score(c.question, gt, gt)); } catch { traps += 1; }
    if (!Number.isFinite(good) || !Number.isFinite(bad)) continue;
    goods.push(good);
    bads.push(bad);
    if (good > bad) wins += 1;
    else if (good === bad) ties += 1;
  }
  return {
    margin: r4(mean(goods) - mean(bads)),
    wins,
    ties,
    traps,
    meanGood: r4(mean(goods)),
    meanBad: r4(mean(bads)),
    worstSelf: selfs.length ? r4(Math.min(...selfs)) : 0,
    stddev: r4(stddev([...goods, ...bads])),
  };
}

// Every probe gets its own instance: a module that traps leaves its allocator in a
// state that fails every later call, which would read as many defects, not one.
async function robustness(scorer) {
  const q = "Is https://example.test/login malicious?";
  const gt = "https://example.test/login is malicious; 12 engines flagged it. Block access.";
  const probes = [
    ["blank answer is exactly 0", (s) => s(q, gt, ""), (v) => v === 0],
    ["whitespace answer is exactly 0", (s) => s(q, gt, "   \t\n  "), (v) => v === 0],
    ["punctuation answer is near 0", (s) => s(q, gt, "... !!! ---"), (v) => v <= 0.05],
    ["self match >= 0.75", (s) => s(q, gt, gt), (v) => v >= 0.75],
    ["76 KB answer stays in [0,1]", (s) => s(q, gt, "malicious ".repeat(8000)), (v) => v >= 0 && v <= 1],
    ["oversized ground truth stays in [0,1]",
      (s) => s(q, `${gt} ${" padding".repeat(9000)}`, "https://example.test/login is malicious."),
      (v) => v >= 0 && v <= 1],
    ["emoji / CJK / RTL stays in [0,1]",
      (s) => s(q, gt, "🚨 https://example.test/login 恶意 مالسيوس malicious, 12 engines. Block access."),
      (v) => v >= 0 && v <= 1],
    ["invalid UTF-8 does not trap",
      (s) => s(q, gt, new Uint8Array([0xff, 0xfe, 0x41, 0x80, 0x6d, 0x61, 0x6c, 0xc0])),
      (v) => v >= 0 && v <= 1],
    ["embedded NULs stay in [0,1]", (s) => s(q, gt, "malicious\0\0\0 12 engines"), (v) => v >= 0 && v <= 1],
  ];

  const results = [];
  for (const [name, run, ok] of probes) {
    try {
      const value = run(await scorer.fresh());
      results.push([name, Number.isFinite(value) && ok(value), r4(value)]);
    } catch (e) {
      results.push([name, false, `trap: ${String(e.message).slice(0, 40)}`]);
    }
  }

  let seed = 20260824;
  const rand = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
  const make = (n) => {
    const b = new Uint8Array(Math.floor(rand() * n));
    for (let k = 0; k < b.length; k += 1) b[k] = Math.floor(rand() * 256);
    return b;
  };
  // A trap leaves the allocator unusable, so the instance is replaced after one
  // rather than before every case: same counts, without 200 instantiations of a
  // module carrying 22 MB of weights.
  let traps = 0;
  let outOfRange = 0;
  let run = await scorer.fresh();
  for (let i = 0; i < FUZZ_CASES; i += 1) {
    try {
      const v = run(make(200), make(400), make(400));
      if (!(v >= 0 && v <= 1) || Number.isNaN(v)) outOfRange += 1;
    } catch {
      traps += 1;
      run = await scorer.fresh();
    }
  }
  results.push([`${FUZZ_CASES} random-byte triples`, traps === 0 && outOfRange === 0,
    `traps=${traps} out-of-range=${outOfRange}`]);
  return results;
}

const [candidatePath, baselinePath] = process.argv.slice(2);
const benchPath = process.env.BENCH ?? "bench/url-scan.json";
const cases = JSON.parse(await readFile(benchPath, "utf8"));
const list = Array.isArray(cases) ? cases : cases.cases;

const candidate = await load(candidatePath);
const baseline = await load(baselinePath);

const a = ordering(candidate, list);
const b = ordering(baseline, list);

console.log(`\nagainst the Canonical Script  (${benchPath.split("/").pop()}, ${list.length} cases)\n`);
console.log(pad("metric", 20), pad(candidate.name, 28), baseline.name);
for (const [label, key] of [["candidate_margin", "margin"], ["mean_good", "meanGood"],
  ["mean_bad", "meanBad"], ["worst_self_match", "worstSelf"], ["score_stddev", "stddev"]]) {
  console.log(pad(label, 20), pad(a[key], 28), b[key]);
}
console.log(pad("ordering wins", 20), pad(`${a.wins}/${list.length}`, 28), `${b.wins}/${list.length}`);
if (a.traps || b.traps) console.log(pad("traps", 20), pad(a.traps, 28), b.traps);

const robustA = await robustness(candidate);
const robustB = await robustness(baseline);
console.log("\nrobustness probes");
for (let i = 0; i < robustA.length; i += 1) {
  const [name, okA, vA] = robustA[i];
  const [, okB, vB] = robustB[i];
  console.log(`  ${pad(name, 38)} ${okA ? "pass" : "FAIL"} ${pad(vA, 22)} ${okB ? "pass" : "FAIL"} ${vB}`);
}

const failed = robustA.filter(([, ok]) => !ok);
if (failed.length) {
  console.log(`\n${candidate.name} fails ${failed.length} robustness probe(s)`);
  process.exit(1);
}
if (a.wins < b.wins) {
  console.log(`\n${candidate.name} orders fewer cases correctly than the Canonical Script`);
  process.exit(1);
}
console.log(`\n${candidate.name}: ${a.wins}/${list.length} ordering wins against the baseline's ${b.wins}`);
