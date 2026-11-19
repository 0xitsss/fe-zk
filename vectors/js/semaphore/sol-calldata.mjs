// Calldata files for Semaphore's own SemaphoreVerifier.sol (gas-run format: `sig:` line + hex words) for
// every proof of vectors/semaphore/{depths.json, examples/*.json}: `valid_<name>.cd` and, for the
// negative control, `bad_<name>.cd` (nullifier + 1).
//   node sol-calldata.mjs <vectors/semaphore dir> <outdir>
// verifyProof(uint256[2] pA, uint256[2][2] pB, uint256[2] pC, uint256[4] pubSignals, uint256 depth)
// with SemaphoreProof.points = [a0, a1, b00, b01, b10, b11, c0, c1] passed through as Semaphore.sol does,
// pubSignals = [merkleTreeRoot, nullifier, hash(message), hash(scope)], hash(x) = keccak256(x) >> 8.
import { keccak256, toBeHex } from "ethers"
import fs from "node:fs"

const [dir, out] = process.argv.slice(2)
const hash = (m) => BigInt(keccak256(toBeHex(BigInt(m), 32))) >> 8n
const cases = []
for (const d of JSON.parse(fs.readFileSync(`${dir}/depths.json`, "utf8"))) cases.push([`depth${d.depth}`, d])
for (const f of ["vote", "claim", "signal"]) {
    const j = JSON.parse(fs.readFileSync(`${dir}/examples/${f}.json`, "utf8"))
    for (const [k, p] of Object.entries(j.proofs)) cases.push([`${f}_${k}`, p])
}
fs.mkdirSync(out, { recursive: true })
const words = (p, nul) =>
    [...p.points, p.merkleTreeRoot, nul, hash(p.message), hash(p.scope), p.merkleTreeDepth].map((n) =>
        BigInt(n).toString(16).padStart(64, "0")
    )
const sig = "sig:verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[4],uint256)"
for (const [name, p] of cases) {
    // order: a(2) b(4) c(2) pub(4) depth; points are already a, b (EVM order), c
    const w = words(p, BigInt(p.nullifier))
    const [a0, a1, b0, b1, b2, b3, c0, c1, root, nul, msg, scope, depth] = w
    const order = (n) => [a0, a1, b0, b1, b2, b3, c0, c1, root, n, msg, scope, depth].join("")
    fs.writeFileSync(`${out}/valid_${name}.cd`, sig + "\n" + order(nul) + "\n")
    fs.writeFileSync(
        `${out}/bad_${name}.cd`,
        sig + "\n" + order((BigInt(p.nullifier) + 1n).toString(16).padStart(64, "0")) + "\n"
    )
}
console.log(cases.map(([n]) => n).join("\n"))
