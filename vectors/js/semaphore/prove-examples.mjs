// Semaphore v4 proofs for the fe-zk semaphore ingot and the anon_* examples, made with the
// official SDK (@semaphore-protocol/proof 4.14.5 generateProof / verifyProof) and the real
// ceremony artifacts. All identities are throw-away test identities.
//   node prove-examples.mjs <artifact-dir> <outdir>
// <artifact-dir> holds semaphore-<d>.wasm / semaphore-<d>.zkey, d = 1..10 (see
// scripts/regen-semaphore-examples.sh, sha256 pinned there).
// Writes <outdir>/depths.json and <outdir>/examples/{vote,claim,signal}.json
import { Group } from "@semaphore-protocol/group"
import { Identity } from "@semaphore-protocol/identity"
import { generateProof, verifyProof } from "@semaphore-protocol/proof"
import { keccak256, toUtf8Bytes } from "ethers"
import fs from "node:fs"
import assert from "node:assert"

const [artDir, outDir] = process.argv.slice(2)
const ident = (label) => new Identity(`fe-zk semaphore test identity ${label}`)
const artifacts = (d) => ({ wasm: `${artDir}/semaphore-${d}.wasm`, zkey: `${artDir}/semaphore-${d}.zkey` })

async function prove(identity, group, message, scope, depth) {
    const p = await generateProof(identity, group, message, scope, depth, artifacts(depth))
    assert.equal(await verifyProof(p), true, "SDK rejects its own proof")
    assert.equal(p.merkleTreeDepth, depth)
    assert.equal(p.points.length, 8)
    return {
        merkleTreeDepth: p.merkleTreeDepth,
        merkleTreeRoot: p.merkleTreeRoot,
        nullifier: p.nullifier,
        message: p.message,
        scope: p.scope,
        points: p.points
    }
}

const w = (file, v) => {
    fs.mkdirSync(file.slice(0, file.lastIndexOf("/")), { recursive: true })
    fs.writeFileSync(file, JSON.stringify(v, null, 2) + "\n")
}

// one proof per depth 1..10; the group has 2^(d-1)+1 members, so its own tree depth is d (d=1: 2 members)
const depths = []
for (let d = 1; d <= 10; d++) {
    const size = d === 1 ? 2 : 2 ** (d - 1) + 1
    const members = Array.from({ length: size }, (_, i) => ident(`depth ${d} member ${i}`))
    const group = new Group(members.map((m) => m.commitment))
    const me = members[size - 1]
    const proof = await prove(me, group, BigInt(d), 1000n + BigInt(d), d)
    depths.push({ depth: d, groupSize: size, ...proof })
    console.log("depth", d, "ok")
}
w(`${outDir}/depths.json`, depths)

const group = (n, tag) => new Group(Array.from({ length: n }, (_, i) => ident(`${tag} ${i}`).commitment))
const members = (n, tag) => Array.from({ length: n }, (_, i) => ident(`${tag} ${i}`))

// vote: poll 1001, depth 3, voters A, B, C (indices 0..2); options 1 and 2
{
    const ms = members(3, "vote voter")
    const g = new Group(ms.map((m) => m.commitment))
    const outsiderGroup = new Group([ident("vote outsider 0").commitment, ident("vote outsider 1").commitment])
    const scope = 1001n
    const proofs = {
        a_option1: await prove(ms[0], g, 1n, scope, 3),
        a_option2: await prove(ms[0], g, 2n, scope, 3),
        b_option2: await prove(ms[1], g, 2n, scope, 3),
        a_option9: await prove(ms[0], g, 9n, scope, 3),
        outsider_option1: await prove(ident("vote outsider 0"), outsiderGroup, 1n, scope, 3)
    }
    w(`${outDir}/examples/vote.json`, {
        depth: 3,
        scope: scope.toString(),
        members: ms.map((m) => m.commitment.toString()),
        root: g.root.toString(),
        proofs
    })
}

// claim: campaign 7, depth 4, allow-list of 5; recipients are addresses
{
    const ms = members(5, "claim holder")
    const g = new Group(ms.map((m) => m.commitment))
    const scope = 7n
    const r1 = 0xa11ce00000000000000000000000000000000001n
    const r2 = 0xb0b0000000000000000000000000000000000002n
    const r3 = 0xca401000000000000000000000000000000000003n
    const proofs = {
        a_to_r1: await prove(ms[0], g, r1, scope, 4),
        b_to_r2: await prove(ms[3], g, r2, scope, 4),
        a_to_r3: await prove(ms[0], g, r3, scope, 4),
        a_to_wide: await prove(ms[0], g, 1n << 160n, scope, 4)
    }
    w(`${outDir}/examples/claim.json`, {
        depth: 4,
        scope: scope.toString(),
        recipients: { r1: r1.toString(), r2: r2.toString(), r3: r3.toString() },
        members: ms.map((m) => m.commitment.toString()),
        root: g.root.toString(),
        proofs
    })
}

// signal: topics 99 and 100, depth 2, open group of 3
{
    const ms = members(3, "signal member")
    const g = new Group(ms.map((m) => m.commitment))
    const h1 = BigInt(keccak256(toUtf8Bytes("the pool is open")))
    const h2 = BigInt(keccak256(toUtf8Bytes("the pool is closed")))
    const proofs = {
        a_t99_h1: await prove(ms[0], g, h1, 99n, 2),
        a_t99_h2: await prove(ms[0], g, h2, 99n, 2),
        a_t100_h1: await prove(ms[0], g, h1, 100n, 2),
        b_t99_h2: await prove(ms[1], g, h2, 99n, 2)
    }
    w(`${outDir}/examples/signal.json`, {
        depth: 2,
        topics: { t99: "99", t100: "100" },
        texts: { h1: h1.toString(), h2: h2.toString() },
        members: ms.map((m) => m.commitment.toString()),
        root: g.root.toString(),
        proofs
    })
}
console.log("done")
process.exit(0)
