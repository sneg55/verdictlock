// Regenerates bench/report.json: every corpus, against both references, from the
// harnesses themselves rather than by hand. The README quotes this file, so it is
// written by the same code that prints the tables.
//
//   node harness/report.mjs <module.wasm> <champion.wasm> [baseline.wasm]
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";

const CORPORA = [
  "bench/url-scan.json",
  "bench/gate-stress.json",
  "bench/external/benchmark.json",
  "bench/external/family-authenticity.json",
  "bench/external/family-numeric.json",
  "bench/external/family-reference.json",
];

const [candidate, champion, baseline] = process.argv.slice(2);
if (!candidate || !champion) {
  console.error("usage: node harness/report.mjs <module.wasm> <champion.wasm> [baseline.wasm]");
  process.exit(2);
}

const json = (script, args, bench) =>
  JSON.parse(execFileSync("node", [script, ...args], {
    env: { ...process.env, JSON: "1", BENCH: bench },
    encoding: "utf8",
    maxBuffer: 64 << 20,
  }));

const report = {};
for (const bench of CORPORA) {
  const name = bench.split("/").pop();
  process.stderr.write(`${name} `);
  report[name] = { champion: json("harness/run.mjs", [candidate, champion], bench) };
  if (baseline) {
    report[name].baseline = json("harness/baseline.mjs", [candidate, baseline], bench);
    process.stderr.write("+baseline ");
  }
  process.stderr.write("\n");
}

writeFileSync("bench/report.json", `${JSON.stringify(report, null, 2)}\n`);
console.error(`wrote bench/report.json (${CORPORA.length} corpora${baseline ? ", both references" : ""})`);
