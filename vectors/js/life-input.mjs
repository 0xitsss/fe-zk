// Builds the witness input for examples/life/circuit/life.circom from packed boards, with
// circomlibjs Poseidon (independent of fe-zk code). Deterministic.
// usage: node life-input.mjs <start_hex> <end_hex> <input.json> <boards.json>
import { buildPoseidon } from "circomlibjs";
import fs from "node:fs";

const [start, end] = [BigInt(process.argv[2]), BigInt(process.argv[3])];
const poseidon = await buildPoseidon();
const commit = (b) => poseidon.F.toString(poseidon([b & ((1n << 128n) - 1n), b >> 128n]));
const bits = (b) => Array.from({ length: 256 }, (_, i) => ((b >> BigInt(i)) & 1n).toString());
const hex = (b) => "0x" + b.toString(16).padStart(64, "0");
const input = { startCommit: commit(start), endCommit: commit(end), start: bits(start) };
fs.writeFileSync(process.argv[4], JSON.stringify(input));
fs.writeFileSync(
  process.argv[5],
  JSON.stringify(
    { start: hex(start), end: hex(end), startCommit: input.startCommit, endCommit: input.endCommit },
    null,
    1,
  ) + "\n",
);
