// Real-ceremony Semaphore v4 fixture generator (depth D).
//   node prove.mjs <depth> <semaphore-D.wasm> <semaphore-D.zkey> <verification-keys.json> <outdir>
// Mirrors @semaphore-protocol/proof 4.14.5 `generateProof` / `verifyProof` (same input layout, same hash()),
// but calls snarkjs directly so the output is a plain snarkjs proof + public signals.
// Writes: verification_key.json (slice of Semaphore's published per-depth key, == `snarkjs zkey export
// verificationkey` of the ceremony zkey, asserted), proof.json, public.json, inputs.json (public part only).
import { Group } from "@semaphore-protocol/group"
import { Identity } from "@semaphore-protocol/identity"
import { keccak256, toBeHex } from "ethers"
import * as snarkjs from "snarkjs"
import fs from "node:fs"
import assert from "node:assert"

const [depthS, wasm, zkey, vkAllPath, outDir] = process.argv.slice(2)
const depth = Number(depthS)
const hash = (m) => (BigInt(keccak256(toBeHex(m, 32))) >> 8n).toString()

// Public, throw-away test identity and group (no secret value of any real user).
const identity = new Identity("fe-zk real-semaphore test identity")
const others = [1n, 2n, 3n, 4n].map((i) => new Identity(`fe-zk real-semaphore member ${i}`).commitment)
const group = new Group([...others.slice(0, 2), identity.commitment, ...others.slice(2)])
const merkleProof = group.generateMerkleProof(group.indexOf(identity.commitment))
const siblings = [...merkleProof.siblings]
while (siblings.length < depth) siblings.push(0n)
const message = 1n
const scope = 42n

const { proof, publicSignals } = await snarkjs.groth16.fullProve(
    {
        secret: identity.secretScalar,
        merkleProofLength: merkleProof.siblings.length,
        merkleProofIndex: merkleProof.index,
        merkleProofSiblings: siblings,
        scope: hash(scope),
        message: hash(message)
    },
    wasm,
    zkey
)
// publicSignals = [merkleRoot, nullifier, message, scope] (circuit outputs first, then the public inputs)
assert.equal(publicSignals[0], merkleProof.root.toString())
assert.equal(publicSignals[2], hash(message))
assert.equal(publicSignals[3], hash(scope))

// Semaphore's per-depth key: shared alpha/beta/gamma, per-depth delta and IC (see verify-proof.ts).
const all = JSON.parse(fs.readFileSync(vkAllPath, "utf8"))
const vk = { ...all, vk_delta_2: all.vk_delta_2[depth - 1], IC: all.IC[depth - 1] }
const vkZkey = await snarkjs.zKey.exportVerificationKey(zkey)
assert.deepStrictEqual(vkZkey, vk, "published verification key differs from the ceremony zkey")
assert.equal(await snarkjs.groth16.verify(vk, publicSignals, proof), true)
const bad = [...publicSignals]
bad[1] = (BigInt(bad[1]) + 1n).toString()
assert.equal(await snarkjs.groth16.verify(vk, bad, proof), false)

fs.mkdirSync(outDir, { recursive: true })
const w = (n, v) => fs.writeFileSync(`${outDir}/${n}`, JSON.stringify(v, null, 2) + "\n")
w("verification_key.json", vk)
w("proof.json", proof)
w("public.json", publicSignals)
w("inputs.json", {
    note: "public part of the witness; the identity is a throw-away test identity",
    merkleTreeDepth: depth,
    groupSize: group.size,
    message: message.toString(),
    scope: scope.toString()
})
console.log("ok: depth", depth, "public", publicSignals)
process.exit(0)
