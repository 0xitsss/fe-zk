// Generates src/hash/poseidon_constants.fe (circomlib Poseidon, BN254, t = 2..6) and the
// reference vectors vectors/poseidon/poseidon.json from circomlibjs (version pinned in
// vectors/js/package-lock.json). Run through scripts/regen-vectors.sh.
//
// Constants come from circomlibjs' "optimized" (sparse matrix) tables: C, S, M, P.
// Matrices are flattened row-major: M[j][i] is at index j * t + i.

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "..");
const nm = path.join(root, "vectors/js/node_modules/circomlibjs");
const opt = JSON.parse(fs.readFileSync(path.join(nm, "src/poseidon_constants_opt.json"), "utf8"));
const pkg = JSON.parse(fs.readFileSync(path.join(nm, "package.json"), "utf8"));
const { buildPoseidon } = await import(
  path.join(root, "vectors/js/node_modules/circomlibjs/main.js")
);

const N_ROUNDS_P = [56, 57, 56, 60, 60];
const hex = (s) => "0x" + BigInt(s).toString(16);

let out = `//! Poseidon (circomlib) round constants for BN254, t = 2..6 (1..5 inputs).
//!
//! GENERATED FILE, do not edit. Produced by \`scripts/gen-poseidon-constants.js\` from
//! circomlibjs ${pkg.version} (\`poseidon_constants_opt.json\`, the optimized sparse-matrix form of
//! the circomlib constants, parameters from the Poseidon reference \`generate_parameters_grain.sage\`).
//! \`C_t\`: round constants, \`S_t\`: sparse-matrix constants of the partial rounds, \`M_t\`: MDS
//! matrix, \`P_t\`: pre-sparse matrix. Matrices are flattened row-major: \`M[j][i]\` is at \`j * t + i\`.

`;
for (let t = 2; t <= 6; t++) {
  const k = t - 2;
  const nP = N_ROUNDS_P[k];
  const C = opt.C[k], S = opt.S[k], M = opt.M[k].flat(), P = opt.P[k].flat();
  if (C.length !== t * 8 + nP || S.length !== (2 * t - 1) * nP || M.length !== t * t || P.length !== t * t)
    throw new Error("unexpected constant shape for t=" + t);
  const emit = (name, arr) => {
    out += `pub const POSEIDON_${name}${t}: [u256; ${arr.length}] = [\n`;
    for (const v of arr) out += `    ${hex(v)},\n`;
    out += `]\n\n`;
  };
  emit("C", C);
  emit("S", S);
  emit("M", M);
  emit("P", P);
}
fs.mkdirSync(path.join(root, "src/hash"), { recursive: true });
fs.writeFileSync(path.join(root, "src/hash/poseidon_constants.fe"), out.trimEnd() + "\n");

// Reference vectors from circomlibjs `poseidon` (not from our code).
const poseidon = await buildPoseidon();
const F = poseidon.F;
const inputsFor = [
  [[1], [0], [123456789]],
  [[1, 2], [0, 0], [3, 4]],
  [[1, 2, 3], [0, 0, 0], [7, 8, 9]],
  [[1, 2, 3, 4], [0, 0, 0, 0], [10, 20, 30, 40]],
  [[1, 2, 3, 4, 5], [0, 0, 0, 0, 0], [11, 22, 33, 44, 55]],
];
const R = 21888242871839275222246405745257275088548364400416034343698204186575808495617n;
const vectors = [];
for (const cases of inputsFor) {
  for (const inp of cases) {
    vectors.push({
      inputs: inp.map(String),
      output: F.toString(poseidon(inp)),
    });
  }
}
// large canonical inputs
vectors.push({
  inputs: [(R - 1n).toString(), (R - 2n).toString()],
  output: F.toString(poseidon([R - 1n, R - 2n])),
});
fs.mkdirSync(path.join(root, "vectors/poseidon"), { recursive: true });
fs.writeFileSync(
  path.join(root, "vectors/poseidon/poseidon.json"),
  JSON.stringify(
    { source: `circomlibjs ${pkg.version} poseidon`, field: "bn254 Fr", vectors },
    null,
    1,
  ) + "\n",
);
console.log("poseidon constants and vectors written");
