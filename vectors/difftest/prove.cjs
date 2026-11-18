// Fresh witnesses + proofs with snarkjs for the differential test (valid cases).
// usage: node prove.cjs <groth16|plonk> <multiplier|membership> <wasm> <zkey> <n> <seed> <node_modules_dir>
// stdout: one JSON object per line {"proof": <snarkjs proof.json>, "pub": [decimal strings]}.
// Inputs are random (seeded); everything else comes from snarkjs / circomlibjs.
const path = require("path");
const [kind, circuit, wasm, zkey, nStr, seedStr, NM] = process.argv.slice(2);
const snarkjs = require(path.join(NM, "snarkjs"));
const R = 21888242871839275222246405745257275088548364400416034343698204186575808495617n;

// splitmix64 on BigInt: deterministic, independent of the platform
let st = BigInt(seedStr) & ((1n << 64n) - 1n);
function next64() {
  st = (st + 0x9e3779b97f4a7c15n) & ((1n << 64n) - 1n);
  let z = st;
  z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & ((1n << 64n) - 1n);
  z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & ((1n << 64n) - 1n);
  return z ^ (z >> 31n);
}
function rnd(bits) {
  let v = 0n;
  for (let i = 0; i < Math.ceil(bits / 64); i++) v = (v << 64n) | next64();
  return v & ((1n << BigInt(bits)) - 1n);
}
function rndField() { return rnd(256) % R; }

async function main() {
  const n = parseInt(nStr);
  let H = null;
  if (circuit === "membership") {
    const { buildPoseidon } = require(path.join(NM, "circomlibjs"));
    const p = await buildPoseidon();
    H = (xs) => p.F.toObject(p(xs));
  }
  for (let k = 0; k < n; k++) {
    let input;
    if (circuit === "multiplier") {
      // mix of small, medium and full-size field elements
      const bits = [8, 64, 128, 253][k % 4];
      input = { a: (rnd(bits) % R).toString(), b: (rnd(bits) % R).toString() };
    } else {
      const DEPTH = 10, size = 1 << DEPTH;
      const secret = rndField(), index = Number(rnd(10)), ext = rndField();
      let level = [];
      for (let i = 0; i < size; i++) level.push(i === index ? H([secret]) : H([BigInt(i + 1000)]));
      const pathElements = [], pathIndices = [];
      let idx = index;
      for (let d = 0; d < DEPTH; d++) {
        pathElements.push(level[idx ^ 1].toString());
        pathIndices.push(idx & 1);
        const nx = [];
        for (let i = 0; i < level.length; i += 2) nx.push(H([level[i], level[i + 1]]));
        level = nx;
        idx >>= 1;
      }
      input = { secret: secret.toString(), pathElements, pathIndices, root: level[0].toString(), externalNullifier: ext.toString() };
    }
    const { proof, publicSignals } = await snarkjs[kind].fullProve(input, wasm, zkey);
    console.log(JSON.stringify({ proof, pub: publicSignals }));
  }
  process.exit(0);
}
main().catch((e) => { console.error(e); process.exit(1); });
