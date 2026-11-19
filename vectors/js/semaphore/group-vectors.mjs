// LeanIMT reference vectors from @semaphore-protocol/group 4.14.5 (zk-kit lean-imt 2.2.5, poseidon-lite 0.3.0).
//   cd vectors/js/semaphore && npm ci --ignore-scripts && node group-vectors.mjs
// Writes ../../semaphore/lean_imt.json. Leaves are Semaphore identity commitments from fixed public seeds.
import { Group } from "@semaphore-protocol/group"
import { Identity } from "@semaphore-protocol/identity"
import { keccak256, toBeHex, toUtf8Bytes } from "ethers"
import fs from "node:fs"

const SIZES = [1, 2, 3, 4, 5, 6, 7, 8, 9, 16, 17, 1024]
const N = Math.max(...SIZES)
const leaves = []
// first 17 leaves are identity commitments; leaf i >= 17 is the plain integer i + 1 (keeps the 1024 case small)
for (let i = 0; i < N; i++) leaves.push(i < 17 ? new Identity(`fe-zk anon leaf ${i}`).commitment : BigInt(i + 1))

const group = new Group()
const roots = []
const depths = []
for (let i = 0; i < N; i++) {
    group.addMember(leaves[i])
    roots.push(group.root)
    depths.push(group.depth)
}
const cases = SIZES.map((n) => ({ size: n, depth: depths[n - 1], root: roots[n - 1].toString() }))

// Semaphore v4 `_hash(message)` = uint256(keccak256(abi.encodePacked(message))) >> 8
const h2f = (m) => (BigInt(keccak256(toBeHex(m, 32))) >> 8n).toString()
const hashInputs = [0n, 1n, 2n, 42n, 1n << 255n, (1n << 256n) - 1n, BigInt(keccak256(toUtf8Bytes("vote:42")))]
const hashToField = hashInputs.map((x) => ({ input: x.toString(), output: h2f(x) }))

fs.writeFileSync(
    new URL("../../semaphore/lean_imt.json", import.meta.url),
    JSON.stringify(
        {
            tool: "@semaphore-protocol/group 4.14.5 / identity 4.14.5",
            leaf_seed: "i < 17: Identity('fe-zk anon leaf <i>').commitment; i >= 17: i + 1",
            leaves: leaves.slice(0, 17).map(String),
            cases,
            roots_first_17: roots.slice(0, 17).map(String),
            hash_to_field: hashToField
        },
        null,
        1
    ) + "\n"
)
console.log("ok", cases.map((c) => `${c.size}:d${c.depth}`).join(" "))
