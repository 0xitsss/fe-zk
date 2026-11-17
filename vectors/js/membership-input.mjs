// Builds the witness input for examples/membership/circuit/membership.circom (depth 10)
// with circomlibjs Poseidon (independent of fe-zk code). Deterministic.
// usage: node membership-input.mjs <out.json>
import { buildPoseidon } from "circomlibjs";
import fs from "node:fs";

const DEPTH = 10;
const poseidon = await buildPoseidon();
const F = poseidon.F;
const H = (xs) => F.toObject(poseidon(xs));

const secret = 1234567890123456789n;
const externalNullifier = 42n;
const index = 5; // leaf position; 2^10 leaves, others are Poseidon(i+1000)

const n = 1 << DEPTH;
let level = [];
for (let i = 0; i < n; i++) level.push(i === index ? H([secret]) : H([BigInt(i + 1000)]));

const pathElements = [];
const pathIndices = [];
let idx = index;
for (let d = 0; d < DEPTH; d++) {
  pathElements.push(level[idx ^ 1].toString());
  pathIndices.push(idx & 1);
  const next = [];
  for (let i = 0; i < level.length; i += 2) next.push(H([level[i], level[i + 1]]));
  level = next;
  idx >>= 1;
}
const input = {
  secret: secret.toString(),
  pathElements,
  pathIndices,
  root: level[0].toString(),
  externalNullifier: externalNullifier.toString(),
};
fs.writeFileSync(process.argv[2], JSON.stringify(input, null, 2) + "\n");
