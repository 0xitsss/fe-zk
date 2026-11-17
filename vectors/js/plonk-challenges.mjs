// Replays the snarkjs 0.7.6 PLONK verifier (JS) on a fixture and records its debug values:
// beta, gamma, alpha, xi, v (= v1), u, PI(xi), r0 as hex. Independent reference for the Fe
// transcript test. Usage: node plonk-challenges.mjs <fixture dir>  (writes challenges.json)
import * as snarkjs from "snarkjs";
import fs from "node:fs";
import path from "node:path";

const dir = process.argv[2];
const rd = (f) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
const out = {};
const logger = {
  info() {},
  warn() {},
  error(m) {
    console.error(m);
  },
  debug(m) {
    const mm = /^(beta|gamma|alpha|xi|u|PI\(xi\)|r0): ([0-9a-f]+)$/.exec(m);
    if (mm) out[mm[1]] = "0x" + mm[2];
    const mv = /^v: ([0-9a-f]+)$/.exec(m);
    if (mv && out.v === undefined) out.v = "0x" + mv[1];
  },
};
const ok = await snarkjs.plonk.verify(
  rd("verification_key.json"),
  rd("public.json"),
  rd("proof.json"),
  logger,
);
if (!ok) throw new Error("proof does not verify");
fs.writeFileSync(path.join(dir, "challenges.json"), JSON.stringify(out, null, 1) + "\n");
console.log(out);
process.exit(0);
