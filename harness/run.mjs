// Loads a scoring module the way a Telegraph validator does (no host imports,
// strings written through the module's own alloc) and reports the metrics the
// node records on a registration.
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";

const REQUIRED_EXPORTS = ["alloc", "dealloc", "rank_answer", "memory"];
const SELF_MATCH_FLOOR = 0.75;

class Scorer {
  constructor(name, bytes, instance) {
    this.name = name;
    this.bytes = bytes;
    this.exports = instance.exports;
  }

  static async load(name, bytes) {
    const module = new WebAssembly.Module(bytes);
    const imports = WebAssembly.Module.imports(module);
    const exported = WebAssembly.Module.exports(module).map((e) => e.name);
    const missing = REQUIRED_EXPORTS.filter((e) => !exported.includes(e));
    const instance = await WebAssembly.instantiate(module, {});
    const scorer = new Scorer(name, bytes, instance);
    scorer.structure = { imports: imports.map((i) => `${i.module}.${i.name}`), missing, exported };
    return scorer;
  }

  put(value) {
    const bytes = value instanceof Uint8Array ? value : new TextEncoder().encode(value);
    const pointer = this.exports.alloc(bytes.length);
    new Uint8Array(this.exports.memory.buffer, pointer, bytes.length).set(bytes);
    return [pointer, bytes.length];
  }

  score(question, groundTruth, answer) {
    const [qp, ql] = this.put(question);
    const [gp, gl] = this.put(groundTruth);
    const [ap, al] = this.put(answer);
    const value = this.exports.rank_answer(qp, ql, gp, gl, ap, al);
    this.exports.dealloc(ap, al);
    this.exports.dealloc(gp, gl);
    this.exports.dealloc(qp, ql);
    return value;
  }
}

const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);
const stddev = (xs) => {
  if (xs.length < 2) return 0;
  const m = mean(xs);
  return Math.sqrt(xs.reduce((a, x) => a + (x - m) ** 2, 0) / (xs.length - 1));
};
const r4 = (x) => Number(x.toFixed(4));

function benchmark(scorer, cases) {
  const perCase = {};
  const goods = [];
  const bads = [];
  const selfMatches = [];
  let wins = 0;
  let ties = 0;
  let minMargin = Infinity;
  let minMarginCase = null;

  for (const c of cases) {
    const good = scorer.score(c.question, c.ground_truth, c.good);
    const bad = scorer.score(c.question, c.ground_truth, c.bad);
    const self = scorer.score(c.question, c.ground_truth, c.ground_truth);
    perCase[c.id] = { good: r4(good), bad: r4(bad), self: r4(self) };
    goods.push(good);
    bads.push(bad);
    selfMatches.push(self);
    if (good > bad) wins += 1;
    else if (good === bad) ties += 1;
    if (good - bad < minMargin) {
      minMargin = good - bad;
      minMarginCase = c.id;
    }
  }

  return {
    module: scorer.name,
    comparable_cases: cases.length,
    wins,
    ties,
    losses: cases.length - wins - ties,
    candidate_margin: r4(mean(goods) - mean(bads)),
    mean_good: r4(mean(goods)),
    mean_bad: r4(mean(bads)),
    worst_self_match: r4(Math.min(...selfMatches)),
    score_stddev: r4(stddev([...goods, ...bads])),
    min_case_margin: r4(minMargin),
    min_case_margin_id: minMarginCase,
    per_case: perCase,
  };
}

function attacks(scorer, cases) {
  const rows = [];
  let passed = 0;
  for (const c of cases) {
    const honest = scorer.score(c.question, c.ground_truth, c.honest);
    const attack = scorer.score(c.question, c.ground_truth, c.attack);
    let ok;
    if (c.rule === "near_zero") ok = attack <= 0.2 && attack < honest;
    else if (c.rule === "below_honest") ok = attack < honest - 0.15;
    else if (c.rule === "near_honest") ok = attack >= 0.6;
    else throw new Error(`unknown rule ${c.rule}`);
    if (ok) passed += 1;
    rows.push({ name: c.name, rule: c.rule, honest: r4(honest), attack: r4(attack), ok });
  }
  return { passed, total: cases.length, rows };
}

function structural(scorer, cases) {
  const rows = [];
  const check = (name, ok, detail) => rows.push({ name, ok, detail });
  const s = scorer.structure;

  check("no host imports", s.imports.length === 0, s.imports.join(",") || "none");
  check("required exports", s.missing.length === 0, s.missing.join(",") || "all present");

  const q = "Is https://example.test/login malicious?";
  const gt = "https://example.test/login is malicious; 12 engines flagged it. Block access.";

  const blank = scorer.score(q, gt, "");
  check("blank answer is exactly 0", blank === 0, String(blank));
  const spaces = scorer.score(q, gt, "   \t\n  ");
  check("whitespace answer is exactly 0", spaces === 0, String(spaces));
  const punct = scorer.score(q, gt, "... !!! ---");
  check("punctuation answer is near 0", punct <= 0.05, String(r4(punct)));

  const self = scorer.score(q, gt, gt);
  check(`self match >= ${SELF_MATCH_FLOOR}`, self >= SELF_MATCH_FLOOR, String(r4(self)));

  const huge = "malicious ".repeat(8000);
  const hugeScore = scorer.score(q, gt, huge);
  check("76 KB answer stays in [0,1]", hugeScore >= 0 && hugeScore <= 1, String(r4(hugeScore)));

  const hugeGt = scorer.score(q, gt + " padding".repeat(9000), "https://example.test/login is malicious.");
  check("oversized ground truth stays in [0,1]", hugeGt >= 0 && hugeGt <= 1, String(r4(hugeGt)));

  const unicode = scorer.score(q, gt, "🚨 https://example.test/login 恶意 مالسيوس malicious, 12 engines. Block access.");
  check("emoji / CJK / RTL stays in [0,1]", unicode >= 0 && unicode <= 1, String(r4(unicode)));

  const invalid = new Uint8Array([0xff, 0xfe, 0x41, 0x80, 0x6d, 0x61, 0x6c, 0xc0]);
  const invalidScore = scorer.score(q, gt, invalid);
  check("invalid UTF-8 does not trap", invalidScore >= 0 && invalidScore <= 1, String(r4(invalidScore)));

  const nulls = scorer.score(q, gt, "malicious\0\0\0 12 engines");
  check("embedded NULs stay in [0,1]", nulls >= 0 && nulls <= 1, String(r4(nulls)));

  // the one thing a scorer must never do is depend on what it scored before
  const leakA = ["Is https://a.test/x malicious?", "malicious. 12 engines flagged https://a.test/x.", "No, https://a.test/x is clean."];
  const leakB = ["Is the passage AI written?", "No, the passage was written by a human.", "Human, not machine generated."];
  const first = scorer.score(...leakA);
  scorer.score(...leakB);
  scorer.score("x", "y", "z");
  scorer.score(...leakB);
  const again = scorer.score(...leakA);
  check("no state carried between calls", first === again, `${r4(first)} then ${r4(again)}`);

  // random bytes must score, not trap
  let seed = 20260824;
  const rand = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
  let fuzzOk = true;
  let fuzzDetail = "200 cases in [0,1]";
  for (let i = 0; i < 200 && fuzzOk; i += 1) {
    const make = (n) => {
      const b = new Uint8Array(Math.floor(rand() * n));
      for (let k = 0; k < b.length; k += 1) b[k] = Math.floor(rand() * 256);
      return b;
    };
    const v = scorer.score(make(200), make(400), make(400));
    if (!(v >= 0 && v <= 1) || Number.isNaN(v)) {
      fuzzOk = false;
      fuzzDetail = `case ${i} returned ${v}`;
    }
  }
  check("200 random-byte triples stay in [0,1]", fuzzOk, fuzzDetail);

  // Stage 1: a correct answer must score strictly above an unrelated one
  let crossOk = true;
  let crossDetail = `${cases.length} cases`;
  for (let i = 0; i < cases.length && crossOk; i += 1) {
    const c = cases[i];
    const other = cases[(i + 1) % cases.length];
    const self = scorer.score(c.question, c.ground_truth, c.ground_truth);
    const unrelated = scorer.score(c.question, c.ground_truth, other.ground_truth);
    if (!(self > unrelated)) {
      crossOk = false;
      crossDetail = `${c.id}: self ${r4(self)} vs unrelated ${r4(unrelated)}`;
    }
  }
  check("self-match beats unrelated cross-match", crossOk, crossDetail);

  const deterministic = new Set();
  for (let i = 0; i < 5; i += 1) deterministic.add(scorer.score(q, gt, "Block https://example.test/login, 12 engines call it malicious."));
  check("repeat calls are identical", deterministic.size === 1, [...deterministic].map(r4).join("/"));

  const passed = rows.filter((r) => r.ok).length;
  return { passed, total: rows.length, rows };
}

async function loadWasm(path) {
  if (/^https?:/.test(path)) {
    const response = await fetch(path);
    if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
    return new Uint8Array(await response.arrayBuffer());
  }
  return new Uint8Array(await readFile(path));
}

const args = process.argv.slice(2);
if (args.length < 1) {
  console.error("usage: node run.mjs <module.wasm> [other.wasm ...]");
  console.error("env: BENCH=bench/url-scan.json ATTACKS=bench/attacks.json PROBE='q|gt|answer' JSON=1");
  process.exit(2);
}

const benchPath = process.env.BENCH ?? new URL("../bench/url-scan.json", import.meta.url).pathname;
const attackPath = process.env.ATTACKS ?? new URL("../bench/attacks.json", import.meta.url).pathname;

const scorers = [];
for (const path of args) {
  const bytes = await loadWasm(path);
  const name = path.split("/").pop();
  scorers.push({ path, scorer: await Scorer.load(name, bytes), bytes });
}

if (process.env.PROBE) {
  const [q, gt, ma] = process.env.PROBE.split("|");
  for (const { scorer } of scorers) console.log(`${scorer.name}\t${r4(scorer.score(q, gt, ma))}`);
  process.exit(0);
}

const bench = JSON.parse(await readFile(benchPath, "utf8"));
const attackSuite = JSON.parse(await readFile(attackPath, "utf8"));
const report = {};

for (const { path, scorer, bytes } of scorers) {
  const st = structural(scorer, bench.cases);
  const bm = benchmark(scorer, bench.cases);
  const at = attacks(scorer, attackSuite.cases);
  report[scorer.name] = {
    path,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    bytes: bytes.length,
    metrics: bm,
    structural: { passed: st.passed, total: st.total, rows: st.rows },
    attacks: { passed: at.passed, total: at.total, rows: at.rows },
  };
}

if (process.env.JSON) {
  console.log(JSON.stringify(report, null, 2));
  process.exit(0);
}

const names = Object.keys(report);
const pad = (s, n) => String(s).padEnd(n);
const col = Math.max(22, ...names.map((n) => n.length + 2));

console.log(`benchmark: ${benchPath.split("/").pop()}  (${bench.cases.length} cases)`);
console.log("");
const rowsOut = [
  ["metric", ...names],
  ["candidate_margin", ...names.map((n) => report[n].metrics.candidate_margin)],
  ["wins", ...names.map((n) => `${report[n].metrics.wins}/${report[n].metrics.comparable_cases}`)],
  ["mean_good", ...names.map((n) => report[n].metrics.mean_good)],
  ["mean_bad", ...names.map((n) => report[n].metrics.mean_bad)],
  ["worst_self_match", ...names.map((n) => report[n].metrics.worst_self_match)],
  ["score_stddev", ...names.map((n) => report[n].metrics.score_stddev)],
  ["min_case_margin", ...names.map((n) => report[n].metrics.min_case_margin)],
  ["structural gates", ...names.map((n) => `${report[n].structural.passed}/${report[n].structural.total}`)],
  ["attack suite", ...names.map((n) => `${report[n].attacks.passed}/${report[n].attacks.total}`)],
  ["bytes", ...names.map((n) => report[n].bytes)],
];
for (const row of rowsOut) console.log(row.map((cell, i) => pad(cell, i === 0 ? 20 : col)).join(""));

console.log("");
for (const n of names) {
  const failed = [
    ...report[n].structural.rows.filter((r) => !r.ok).map((r) => `structural: ${r.name} (${r.detail})`),
    ...report[n].attacks.rows.filter((r) => !r.ok).map((r) => `attack: ${r.name} rule=${r.rule} honest=${r.honest} attack=${r.attack}`),
  ];
  const per = Object.entries(report[n].metrics.per_case);
  const losses = per.filter(([, v]) => v.good < v.bad);
  const ties = per.filter(([, v]) => v.good === v.bad);
  if (failed.length || losses.length) {
    console.log(`${n} failures:`);
    for (const f of failed) console.log(`  ${f}`);
    for (const [id, v] of losses) console.log(`  ordering: ${id} good=${v.good} bad=${v.bad}`);
  } else {
    console.log(`${n}: all gates pass, ${report[n].metrics.wins}/${report[n].metrics.comparable_cases} ordering wins`);
  }
  // a tie is not a win on the node either, but it is a known miss rather than a
  // regression, so it is reported without failing the run
  for (const [id, v] of ties) console.log(`  ${n} ties on ${id} at ${v.good}, neither answer is separated`);
}

const primary = report[names[0]];
const primaryLosses = Object.values(primary.metrics.per_case).filter((v) => v.good < v.bad).length;
const clean = primary.structural.passed === primary.structural.total
  && primary.attacks.passed === primary.attacks.total
  && primaryLosses === 0;
process.exit(clean ? 0 : 1);
