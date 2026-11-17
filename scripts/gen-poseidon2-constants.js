// Generates src/hash/poseidon2_constants.fe and vectors/poseidon2/*.json.
//
// usage: node gen-poseidon2-constants.js <poseidon2_instance_bn256.rs> <poseidon2.rs> <p3.json>
//   - HorizenLabs/poseidon2 (pinned commit, see scripts/regen-vectors.sh): BN254 t = 3 round
//     constants, internal diagonal minus one, and the known-answer test of the reference.
//   - p3.json: output of vectors/plonky3 (Plonky3 default KoalaBear Poseidon2, width 16).
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const [bnPath, refPath, p3Path] = process.argv.slice(2);
const bnSrc = fs.readFileSync(bnPath, "utf8");
const refSrc = fs.readFileSync(refPath, "utf8");
const p3 = JSON.parse(fs.readFileSync(p3Path, "utf8"));

const hex = (s) => "0x" + BigInt(s).toString(16);

// Extract `pub static ref NAME: ... = vec![ ... ];` and return all from_hex literals as rows.
function rows(name) {
  const start = bnSrc.indexOf(`pub static ref ${name}:`);
  if (start < 0) throw new Error("missing " + name);
  const end = bnSrc.indexOf("];\n", bnSrc.indexOf("vec![", start) + 5);
  const body = bnSrc.slice(start, end);
  // rows are `vec![ from_hex(..), ... ]`, one per inner vec
  const inner = body.split("vec![").slice(2);
  return inner.map((r) => [...r.matchAll(/from_hex\("(0x[0-9a-fA-F]+)"\)/g)].map((m) => m[1]));
}
const rc3 = rows("RC3");
const diag = [...bnSrc.slice(bnSrc.indexOf("MAT_DIAG3_M_1"), bnSrc.indexOf("MAT_INTERNAL3")).matchAll(/from_hex\("(0x[0-9a-fA-F]+)"\)/g)].map((m) => m[1]);
if (rc3.length !== 64 || rc3.some((r) => r.length !== 3)) throw new Error("RC3 shape " + rc3.length);
if (diag.length !== 3) throw new Error("diag shape");
if (!/Poseidon2Params::new\(3, 5, 8, 56,/.test(bnSrc)) throw new Error("unexpected params");

const bnExt = [...rc3.slice(0, 4), ...rc3.slice(60, 64)].flat();
const bnInt = rc3.slice(4, 60).map((r) => r[0]);
// internal rounds only use column 0; the reference file pads the rest with zeros
for (const r of rc3.slice(4, 60)) if (BigInt(r[1]) !== 0n || BigInt(r[2]) !== 0n) throw new Error("non-zero pad");

// reference known-answer test (bn256 module)
const kmod = refSrc.slice(refSrc.indexOf("mod poseidon2_tests_bn256"), refSrc.indexOf("mod poseidon2_tests_pallas"));
const kat = [...kmod.matchAll(/perm\[\d\], from_hex\("(0x[0-9a-fA-F]+)"\)/g)].map((m) => m[1]);
if (kat.length !== 3) throw new Error("kat shape");

const emit = (name, arr, doc) =>
  `${doc}\npub const ${name}: [u256; ${arr.length}] = [\n${arr.map((v) => `    ${hex(v)},\n`).join("")}]\n\n`;

let out = `//! Poseidon2 constants. GENERATED FILE, do not edit: \`scripts/gen-poseidon2-constants.js\`.
//!
//! BN254 t = 3 (x^5, 8 full + 56 partial rounds): HorizenLabs/poseidon2
//! \`plain_implementations/src/poseidon2/poseidon2_instance_bn256.rs\`.
//! KoalaBear t = 16 (x^3, 8 full + 20 partial rounds): Plonky3 \`${p3.source}\`.
//! External round constants are flattened round-major: the 4 initial full rounds, then the 4
//! final full rounds, \`t\` constants each. Internal rounds add a constant to lane 0 only.
//! \`DIAG\` is the diagonal of the internal matrix \`1 + diag(DIAG)\`.

`;
out += emit("P2_BN254_T3_EXT", bnExt, "/// BN254 t = 3 external round constants (8 rounds x 3).");
out += emit("P2_BN254_T3_INT", bnInt, "/// BN254 t = 3 internal (partial) round constants.");
out += emit("P2_BN254_T3_DIAG", diag, "/// BN254 t = 3 internal matrix diagonal minus one.");
const kExt = [...p3.external_initial.flat(), ...p3.external_final.flat()];
out += emit("P2_KOALABEAR_T16_EXT", kExt.map(String), "/// KoalaBear t = 16 external round constants (8 rounds x 16).");
out += emit("P2_KOALABEAR_T16_INT", p3.internal.map(String), "/// KoalaBear t = 16 internal (partial) round constants.");
out += emit("P2_KOALABEAR_T16_DIAG", p3.internal_diag.map(String), "/// KoalaBear t = 16 internal matrix diagonal minus one.");
fs.mkdirSync(path.join(root, "src/hash"), { recursive: true });
fs.writeFileSync(path.join(root, "src/hash/poseidon2_constants.fe"), out.trimEnd() + "\n");

fs.mkdirSync(path.join(root, "vectors/poseidon2"), { recursive: true });
fs.writeFileSync(
  path.join(root, "vectors/poseidon2/bn254_t3.json"),
  JSON.stringify(
    {
      source: "HorizenLabs/poseidon2 plain_implementations known-answer test (bn256, t=3)",
      permutation: [{ input: ["0", "1", "2"], output: kat }],
    },
    null,
    1,
  ) + "\n",
);
fs.writeFileSync(
  path.join(root, "vectors/poseidon2/koalabear_t16.json"),
  JSON.stringify({ source: p3.source, permutation: p3.kats, merkle: p3.merkle }, null, 1) + "\n",
);
console.log("poseidon2 constants and vectors written");
