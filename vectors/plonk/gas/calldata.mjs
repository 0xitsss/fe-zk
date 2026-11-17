// Builds ABI calldata for snarkjs's PlonkVerifier.verifyProof(uint256[24],uint256[N]).
// Usage: node calldata.mjs <fixture dir> <4-byte selector hex>  -> prints hex
import fs from "node:fs";
import path from "node:path";

const dir = process.argv[2];
const sel = process.argv[3];
const pr = JSON.parse(fs.readFileSync(path.join(dir, "proof.json"), "utf8"));
const pub = JSON.parse(fs.readFileSync(path.join(dir, "public.json"), "utf8"));
const w = [];
for (const k of ["A", "B", "C", "Z", "T1", "T2", "T3", "Wxi", "Wxiw"]) w.push(pr[k][0], pr[k][1]);
for (const k of ["eval_a", "eval_b", "eval_c", "eval_s1", "eval_s2", "eval_zw"]) w.push(pr[k]);
const h = (x) => BigInt(x).toString(16).padStart(64, "0");
console.log(sel + w.map(h).join("") + pub.map(h).join(""));
