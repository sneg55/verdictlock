// Prints the parts behind a score for one case id, or for a literal triple.
import { readFile } from "node:fs/promises";

const FIELDS = ["score", "base", "recall", "precision", "trigram", "penalty", "sub_factor",
  "verdict_gap", "confirm_gap", "conflict", "slot_conflict", "target_conflict", "unknown_gap", "direction_gap", "affirm_gap", "axis_support", "bigram"];

const wasmPath = process.argv[2];
const caseId = process.argv[3];
const bytes = new Uint8Array(await readFile(wasmPath));
const { instance } = await WebAssembly.instantiate(bytes, {});
const { alloc, memory, probe } = instance.exports;
if (!probe) throw new Error("build with --features probe");

const put = (value) => {
  const b = new TextEncoder().encode(value);
  const p = alloc(b.length);
  new Uint8Array(memory.buffer, p, b.length).set(b);
  return [p, b.length];
};
const parts = (q, gt, ma) => {
  const out = {};
  for (let f = 0; f < FIELDS.length; f += 1) {
    const [qp, ql] = put(q), [gp, gl] = put(gt), [ap, al] = put(ma);
    out[FIELDS[f]] = Number(probe(f, qp, ql, gp, gl, ap, al).toFixed(4));
  }
  return out;
};

const bench = JSON.parse(await readFile(new URL("../bench/url-scan.json", import.meta.url), "utf8"));
const c = bench.cases.find((x) => x.id === caseId);
if (!c) {
  console.error(`no case ${caseId}; ids: ${bench.cases.map((x) => x.id).join(" ")}`);
  process.exit(2);
}
console.log(`case ${c.id}`);
console.log("good", parts(c.question, c.ground_truth, c.good));
console.log("bad ", parts(c.question, c.ground_truth, c.bad));
