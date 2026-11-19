# Using fe-zk

A practical guide for a Fe developer. The first recipe takes you from nothing to a deployed anonymous-voting contract
in one evening; the next ones turn a circuit and a proof system of your own into an on-chain verifier; the last part
is a reference for the field, curve and hash layer. Every command and Fe file below was run (the last section lists
what was not).

Tested with: `fe 26.5.0 (3dab95f)`, `fe-zk-gen 0.2.0`, Node 24 with `@semaphore-protocol/{group,identity,proof}` 4.14.5,
circom 2.2.2, snarkjs 0.7.6, gnark v0.12.0 (Go), arkworks 0.5.0, Plonky3 0.4.2, Foundry (`anvil`, `cast`) 1.8.3.

fe-zk has **not been audited**; read the [integration checklist](#7-integration-safety-checklist) before you deploy
anything, and [SECURITY.md](../SECURITY.md) for the threat model.

Layout used throughout (the repository and your project are siblings, so the relative path in `fe.toml` works):

```
work/
  fe-zk/        git clone of this repository
  vote/         your Fe project (the first recipe)
```

Compile times are long: `fe test` or `fe build` of a project that depends on `fe_zk` takes 70 to 105 seconds on the
machine used for this guide (an old Xeon without AVX), and about 2.5 minutes once the Semaphore keys are in. Plan for it.

## Contents

1. [Anonymous membership in one evening](#1-anonymous-membership-in-one-evening)
2. [Install](#2-install)
3. [Add fe-zk to your Fe project](#3-add-fe-zk-to-your-fe-project)
4. [Recipes, one per proof system](#4-recipes-one-per-proof-system)
   (a) [circom + snarkjs Groth16](#a-circom--snarkjs-groth16) (b) [arkworks Groth16](#b-arkworks-groth16)
   (c) [snarkjs PLONK](#c-snarkjs-plonk) (d) [gnark PLONK](#d-gnark-plonk) (e) [halo2-KZG](#e-halo2-kzg)
   (f) [KZG openings and EIP-4844](#f-kzg-openings-and-eip-4844-point-evaluation) (g) [FRI/STARK](#g-fristark-plonky3)
5. [Using the library directly](#5-using-the-library-directly)
6. [Compile, test, deploy](#6-compile-test-deploy)
7. [Integration safety checklist](#7-integration-safety-checklist)
8. [Troubleshooting](#8-troubleshooting)
9. [What was run, and what was not](#9-what-was-run-and-what-was-not)

## 1. Anonymous membership in one evening

Goal: a poll where registered members vote once each and nobody learns who voted for what. Semaphore v4 gives the
proofs (made in JavaScript with the official SDK, against the keys of the public Semaphore ceremony), `fe_zk::anon`
gives the group tree and the nullifier registry, and the `fe_zk_semaphore` ingot verifies the proofs on chain.
You write about 50 lines of Fe. Prerequisites: `fe` 26.5.0 ([section 2](#2-install)), Node 20 or newer, and Foundry
for the deploy step.

```sh
mkdir work && cd work
git clone https://github.com/0xitsss/fe-zk
mkdir vote && cd vote
```

**1. `fe.toml`.** Two dependencies. `allow_unsafe = true` is required: fe-zk's precompile wrappers use `unsafe`
blocks and Fe refuses to compile a call into them otherwise ([section 3](#3-add-fe-zk-to-your-fe-project)).

```toml
[ingot]
name = "vote"
version = "0.1.0"

[dependencies]
fe_zk = { path = "../fe-zk", allow_unsafe = true }
fe_zk_semaphore = { path = "../fe-zk/semaphore", allow_unsafe = true }
```

**2. The contract, `src/lib.fe`.** The scope (the poll id) is fixed at deployment; the message of a proof is the
option voted for. `verify_proof_low` supports trees of depth 1 to 5, so groups of up to 32 members
([why, and how to go beyond](#beyond-32-members)).

```fe
use fe_zk::Bn254Fr
use fe_zk::anon::{LeanImt, Nullifiers}
use fe_zk_semaphore::verify_proof_low
use std::evm::{Address, Call, Ctx, StorageMap}

/// verify_proof_low has keys for tree depth 1..5, so a group of at most 32 members.
const MAX_MEMBERS: u256 = 32

msg VoteMsg {
    #[selector = sol("register(uint256)")]
    Register { commitment: u256 },
    #[selector = sol("vote(uint256,uint256,uint256,uint256,uint256[8])")]
    Vote { option: u256, depth: u256, root: u256, nullifier: u256, points: [u256; 8] },
    #[selector = sol("tally(uint256)")]
    Tally { option: u256 } -> u256,
}

struct Poll { owner: Address, scope: u256, options: u256 }

pub contract Vote uses (ctx: Ctx, call: Call) {
    mut tree: LeanImt,
    mut nulls: Nullifiers,
    mut tallies: StorageMap<u256, u256>,
    mut poll: Poll,

    // scope = the poll id the proofs are made for; options are 0..options
    init(scope: u256, options: u256)
    uses (ctx, mut poll)
    {
        poll.owner = ctx.caller()
        poll.scope = scope
        poll.options = options
    }

    recv VoteMsg {
        Register { commitment } uses (ctx, poll, mut tree) {
            assert(ctx.caller() == poll.owner)
            assert(tree.size() < MAX_MEMBERS)
            tree.insert(leaf: Bn254Fr::from_canonical(commitment).unwrap())
        }

        Vote { option, depth, root, nullifier, points }
        uses (call, ctx, poll, tree, mut nulls, mut tallies)
        {
            assert(option < poll.options)
            assert(tree.is_known_root(root: root))
            assert(!nulls.is_spent(scope: poll.scope, nullifier: nullifier))
            // the option is the proof's message, the poll id its scope
            assert(
                verify_proof_low(depth, root, nullifier, option, poll.scope, points),
            )
            assert(nulls.spend(scope: poll.scope, nullifier: nullifier))
            tallies[option] = tallies[option] + 1
        }

        Tally { option } -> u256 uses (tallies) {
            tallies[option]
        }
    }
}
```

The checks run in this order, each before the next costs gas: the option exists, the root is one this poll has had,
the nullifier is unspent for this scope, the proof verifies; only then the nullifier is spent and the vote counted.
`verify_proof_low` returns `false` (never reverts) for a wrong depth, a non-canonical word or a proof that does not
verify; the `assert` turns that into a revert. It declares `uses (call: Call, ctx: Ctx)`, so your handler needs `ctx`
too.

**3. Proofs from the Semaphore SDK.** `package.json` (the exact versions this guide was run with):

```json
{
  "name": "vote-proofs",
  "private": true,
  "type": "module",
  "dependencies": {
    "@semaphore-protocol/group": "4.14.5",
    "@semaphore-protocol/identity": "4.14.5",
    "@semaphore-protocol/proof": "4.14.5"
  }
}
```

`make-fixture.mjs` builds a three-member group, proves two votes and writes both the Fe constants for the test and
`proofs.json` for the deploy step. The SDK downloads the proving key of the group's depth (here 2) from
`snark-artifacts.pse.dev` on first use. The identities are throw-away test identities.

```js
// node make-fixture.mjs > src/fixture.fe     (also writes proofs.json for cast)
import { writeFileSync } from "node:fs"
import { Group } from "@semaphore-protocol/group"
import { Identity } from "@semaphore-protocol/identity"
import { generateProof } from "@semaphore-protocol/proof"

const SCOPE = 42n // the poll id; the contract is deployed with the same number
const alice = new Identity("alice (throw-away test identity)")
const bob = new Identity("bob (throw-away test identity)")
const carol = new Identity("carol (throw-away test identity)")
const group = new Group([alice.commitment, bob.commitment, carol.commitment])

// message = the option voted for. The SDK downloads the proving key of the group's depth on first use.
const a1 = await generateProof(alice, group, 1n, SCOPE)
const b2 = await generateProof(bob, group, 2n, SCOPE)
writeFileSync("proofs.json", JSON.stringify({ members: group.members.map(String), alice: a1, bob: b2 }, null, 2))

const arr = (xs) => "[" + xs.join(", ") + "]"
const proof = (name, p) => `
pub const ${name}_NULLIFIER: u256 = ${p.nullifier}
pub fn ${name.toLowerCase()}_points() -> [u256; 8] {
    ${arr(p.points)}
}`
console.log(`pub const SCOPE: u256 = ${SCOPE}
pub const DEPTH: u256 = ${a1.merkleTreeDepth}
pub const ROOT: u256 = ${a1.merkleTreeRoot}
pub fn members() -> [u256; 3] {
    ${arr(group.members)}
}${proof("ALICE", a1)}${proof("BOB", b2)}`)

process.exit(0) // snarkjs leaves worker threads running; without this node never exits
```

```sh
npm install --no-audit --no-fund
node make-fixture.mjs > src/fixture.fe && fe fmt     # about 3 s after the download
```

**4. Tests, `src/tests.fe`.** The contract is deployed in the Fe test EVM; the proofs are the real ones from step 3.
The second test is the front-running case: the option is the proof's message, so a copied proof with another option
fails and burns nothing.

```fe
use super::{Vote, VoteMsg}
use super::fixture::{
    ALICE_NULLIFIER,
    BOB_NULLIFIER,
    DEPTH,
    ROOT,
    SCOPE,
    alice_points,
    bob_points,
    members,
}
use std::evm::{Address, Evm}

fn register(_ p: Address, _ commitment: u256) -> bool uses (evm: mut Evm) {
    evm.try_call(
        addr: p,
        gas: 30000000,
        value: Wei::zero(),
        message: VoteMsg::Register { commitment: commitment },
    )
        .success()
}

/// A poll with three options and the three members registered (same order as the proofs' group).
fn setup() -> Address uses (evm: mut Evm) {
    let p = evm.create2<Vote>(value: Wei::zero(), args: (SCOPE, 3), salt: 0)
    let ms = members()
    assert(register(p, ms[0]))
    assert(register(p, ms[1]))
    assert(register(p, ms[2]))
    p
}

fn vote(_ p: Address, _ option: u256, _ nullifier: u256, _ points: [u256; 8]) -> bool
uses (evm: mut Evm)
{
    evm.try_call(
        addr: p,
        gas: 30000000,
        value: Wei::zero(),
        message: VoteMsg::Vote {
            option: option,
            depth: DEPTH,
            root: ROOT,
            nullifier: nullifier,
            points: points,
        },
    )
        .success()
}

fn tally(_ p: Address, _ option: u256) -> u256 uses (evm: mut Evm) {
    evm.call(
        addr: p,
        gas: 1000000,
        value: Wei::zero(),
        message: VoteMsg::Tally { option: option },
    )
}

#[test]
fn each_member_votes_once() uses (evm: mut Evm) {
    let p = setup()
    assert(vote(p, 1, ALICE_NULLIFIER, alice_points()))
    assert(vote(p, 2, BOB_NULLIFIER, bob_points()))
    assert(tally(p, 1) == 1)
    assert(tally(p, 2) == 1)
    // the same proof again: the nullifier is spent
    assert(!vote(p, 1, ALICE_NULLIFIER, alice_points()))
    assert(tally(p, 1) == 1)
}

#[test]
fn changing_the_option_breaks_the_proof() uses (evm: mut Evm) {
    let p = setup()
    // a front-runner copies Alice's pending proof and swaps the option: the option is the proof's message
    assert(!vote(p, 2, ALICE_NULLIFIER, alice_points()))
    assert(tally(p, 2) == 0)
    // the failed attempt burned nothing: Alice's own vote still goes through
    assert(vote(p, 1, ALICE_NULLIFIER, alice_points()))
}
```

```sh
fe fmt --check && fe test        # changing_the_option_breaks_the_proof, each_member_votes_once: 2 passed (about 3 minutes)
```

**5. Deploy to a local chain.** `fe build` writes `out/Vote.bin` (creation code, hex without `0x`),
`out/Vote.runtime.bin` (21,173 bytes, under the 24,576 EIP-170 limit) and `out/Vote.abi.json`. The commands below
deploy with a constructor `(scope = 42, options = 3)`, register the three members (only the deployer may), and cast
Alice's vote for option 1.

```sh
fe build                                                          # about 2.5 minutes
anvil --port 18545 &
export RPC=http://127.0.0.1:18545
export KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80    # anvil's public dev key #0

VOTE=$(cast send --rpc-url $RPC --private-key $KEY --create "0x$(cat out/Vote.bin)" "constructor(uint256,uint256)" 42 3 --json | jq -r .contractAddress)
for c in $(jq -r '.members[]' proofs.json); do
  cast send --rpc-url $RPC --private-key $KEY $VOTE "register(uint256)" $c --json | jq -r .status      # 0x1 three times
done

P=alice
cast send --rpc-url $RPC --private-key $KEY $VOTE "vote(uint256,uint256,uint256,uint256,uint256[8])" \
    1 $(jq -r ".$P.merkleTreeDepth" proofs.json) $(jq -r ".$P.merkleTreeRoot" proofs.json) \
    $(jq -r ".$P.nullifier" proofs.json) "[$(jq -r ".$P.points | join(\",\")" proofs.json)]" \
    --gas-limit 1000000 --json | jq -r '.status + " gas " + .gasUsed'                    # 0x1 gas 0x47fd1 (294,865)
cast call --rpc-url $RPC $VOTE "tally(uint256)(uint256)" 1                              # 1
```

Run the same `cast send` again and it fails (the nullifier is spent: status `0x0`). Send Bob's proof (`P=bob`) with
option `1` and it fails too, because his proof was made for option 2; with option `2` it succeeds.

**Gas limit.** One vote costs about 295,000 gas (about 215,000 of it is the Groth16 check). The `--gas-limit` above
is explicit on purpose: contracts that treat a failed inner call as "invalid" cannot tell an invalid proof from a
starved call, so always forward ample gas. The fe-zk verifiers themselves revert when the remaining gas is below what
the next precompile call needs (the *failure rule*, [SAFETY.md](SAFETY.md) section 10), so with this contract
`cast send` without `--gas-limit` also estimated and recorded the vote correctly (tested).

**What the contract does not do for you.** It checks that the root is one *its own* tree has had (`is_known_root`),
which is what ties a proof to this poll's group; it fixes the scope in storage, so a member cannot mint a second
nullifier by choosing another one; and it never uses a proof or its hash as an id (Groth16 proofs are malleable).
Keep these three if you change the contract. The other two examples, `examples/anon_claim` (one claim per member,
recipient bound to the proof) and `examples/anon_signal` (anonymous signals per topic), follow the same pattern.

### Beyond 32 members

The examples cap a group at 32 members (`MAX_MEMBERS`, tree depth 5) for a size reason, not a cryptographic one.
Contract code is limited to 24,576 bytes (EIP-170): a `LeanImt` is about 14 KB, `verify_proof_low` (the five depth
1 to 5 keys) about 7 KB, and `verify_proof` with all ten keys about 11 KB; `LeanImt` plus `verify_proof` does not fit
in one contract. For bigger groups (depth 6 to 10, up to 1,024 members, which is all the ingot ships keys for) keep the
verifier in its own contract using `verify_proof_high` and call it from the contract that holds the tree. This is
tested in a scratch ingot with a real depth 6 proof:

```fe
use fe_zk_semaphore::verify_proof_high
use std::evm::{Call, Ctx}

msg VerifierMsg {
    #[selector = sol("verify(uint256,uint256,uint256,uint256,uint256,uint256[8])")]
    Verify { depth: u256, root: u256, nullifier: u256, message: u256, scope: u256, points: [u256; 8] } -> bool,
}

/// Only the verifier, for tree depth 6..10 (groups of 33 to 1024 members).
pub contract HighVerifier uses (ctx: Ctx, call: Call) {
    recv VerifierMsg {
        Verify { depth, root, nullifier, message, scope, points } -> bool uses (ctx, call) {
            verify_proof_high(depth, root, nullifier, message, scope, points)
        }
    }
}
```

Deploy `HighVerifier` once and pass its address to the group contract. The group contract keeps the `LeanImt`, the
`Nullifiers` and the poll logic, and asks the verifier with a static call, as recipe (a) does:
`let ok: bool = link.verifier.static(VerifierMsg::Verify { depth: depth, root: root, nullifier: nullifier, message: option, scope: scope, points: points })`
(`link` being a storage struct with the verifier's `Address`), then `assert(ok)` and spends the nullifier. Registration needs the same cap logic with a larger bound (`MAX_MEMBERS` of your choice up to
1,024). A group that spans both ranges needs one verifier contract per range (`verify_proof_low` and `verify_proof_high`)
and a choice by `depth`, or a third contract holding `verify_proof` alone (about 11 KB).

### Key-only verifier, no generated contract

`fe-zk-gen groth16 --lib-module` emits only the key (constants, compile-time checks, `pub fn vk()`), so you can call
`groth16::verify_calldata` from your own contract instead of deploying a generated one. This is how the Semaphore ingot
is built. With the multiplier circuit of recipe (a):

```sh
fe-zk/gen/target/release/fe-zk-gen groth16 --snarkjs verification_key.json --lib-module src/key.fe --module-name key
```

```fe
use fe_zk::groth16
use ingot::key::vk
use std::evm::{Call, Ctx}

/// Verifies a snarkjs proof of the circuit the key was exported from. `b` is the JSON order of snarkjs'
/// proof.json: [[x_real, x_imag], [y_real, y_imag]].
pub fn check(_ a: [u256; 2], _ b: [[u256; 2]; 2], _ c: [u256; 2], _ inputs: [u256; 2]) -> bool
uses (call: Call, ctx: Ctx)
{
    groth16::verify_calldata(vk(), a, b, c, inputs)
}
```

`src/key.fe` carries the key digest in its header and `static_assert`s every key point; the library does the rest.
Tested with the repository's multiplier key: a valid proof is accepted and a changed input rejected.

## 2. Install

**Fe 26.5.0.** fe-zk supports exactly one compiler version.

```sh
curl -fsSL https://raw.githubusercontent.com/argotorg/fe/master/feup/feup.sh | bash -s -- --version v26.5.0
fe --version        # fe 26.5.0 (3dab95f)
```

**CPU without AVX: `Illegal instruction`.** The prebuilt binary crashes on such CPUs. Build the same tag from source
(the recipe from [DESIGN.md](DESIGN.md), "Local build"). The build needs the `tree-sitter` CLI on `PATH`, which the
repository's `crates/tree-sitter-fe` provides through npm:

```sh
git clone https://github.com/argotorg/fe && cd fe && git checkout v26.5.0
(cd crates/tree-sitter-fe && npm ci)
PATH="$PWD/crates/tree-sitter-fe/node_modules/.bin:$PATH" cargo build --release --locked -p fe
./target/release/fe --version     # fe 26.5.0 (3dab95f), same version, put it on PATH
```

**Rust** is needed only for the generator `fe-zk-gen` (a Cargo workspace in `fe-zk/gen`, edition 2021, no pinned
`rust-version`; a nightly cargo 1.100 was used here). Build it once and call the binary from anywhere:

```sh
cargo build --release --manifest-path fe-zk/gen/Cargo.toml -p fe-zk-gen
fe-zk/gen/target/release/fe-zk-gen --help
```

The recipes below use `cargo run -p fe-zk-gen -- ...` from `fe-zk/gen`, which is the same program.

**Optional tools**, only for producing proofs and keys (none is needed to run the library tests):

| Tool | Version | For |
|---|---|---|
| circom | 2.2.2 | circuits for recipe (a) and (c) |
| snarkjs | 0.7.6 | Groth16 and PLONK keys, proofs, `soliditycalldata` |
| Go | 1.22 or newer | gnark exporter, recipe (d) |
| Rust + arkworks 0.5.0 | | recipe (b), (e), (g) |
| Node 20+ and `@semaphore-protocol/*` 4.14.5 | | proofs for the first recipe |
| Foundry (`anvil`, `cast`), `jq` | 1.8.3 | deploying and calling, [section 6](#6-compile-test-deploy) |

`scripts/setup-tools.sh` builds circom 2.2.2 from a pinned commit into `$ZK_TOOLS_DIR` (default
`~/.cache/fe-zk-tools`) and runs `npm ci --ignore-scripts` in `vectors/js`, which installs the pinned snarkjs at
`fe-zk/vectors/js/node_modules/.bin/snarkjs`. The recipes use that snarkjs:

```sh
fe-zk/scripts/setup-tools.sh
export PATH="$HOME/.cache/fe-zk-tools/bin:$PWD/fe-zk/vectors/js/node_modules/.bin:$PATH"
circom --version && snarkjs --help | head -1
```

(Run from `work/`. If you already have circom 2.2.x, skip the circom build and only run
`(cd fe-zk/vectors/js && npm ci --ignore-scripts)`.)

## 3. Add fe-zk to your Fe project

A Fe project (an "ingot") is a directory with `fe.toml` and `src/lib.fe`. Add fe-zk as a dependency:

```toml
[ingot]
name = "my_project"
version = "0.1.0"

[dependencies]
fe_zk = { path = "../fe-zk", allow_unsafe = true }
```

- **`allow_unsafe = true` is required.** fe-zk's precompile wrappers use `unsafe` blocks. Without it, the first call
  that reaches one fails to compile with `call reaches unsafe code in 'fe_zk', which is not allowed to use it`
  ([Troubleshooting](#8-troubleshooting)). It trusts the whole `fe_zk` ingot, so pin what you trust.
- **The path must be relative.** Fe rejects absolute paths (`The dependency "fe_zk" has an invalid path ...`) and
  anything with a URL scheme. Count the `..` from the `fe.toml` that contains the line. A generated verifier ingot
  has its own `fe.toml`; `--fe-zk-path` writes that path (it is relative to the output directory).
- **Git form.** Fe 26.5 also reads remote dependencies. The keys, from
  `crates/common/src/config/dependency.rs` of the compiler: `source` (a URL), `rev` and an optional `path` (a
  sub-directory inside the repository; fe-zk's `fe.toml` is at the repository root, so you do not need it). `rev`
  must be a full 40-character commit id (the resolver parses it as a git object id; a branch or short hash is not
  accepted):

  ```toml
  fe_zk = { source = "https://github.com/0xitsss/fe-zk.git", rev = "<full-commit-sha>", allow_unsafe = true }
  ```

  I tested this form offline against a local git repository (`source = "file:///.../gitrepo"` with that
  repository's `HEAD` sha): `fe test` fetched, checked out and compiled it, and the `Field` tests passed. I did not
  test it against GitHub. `--fe-zk-path` writes a filesystem path, so for a generated verifier ingot edit the
  `fe_zk` line of its `fe.toml` into the `source`/`rev` form by hand (not tested).

Generated verifier ingots are ordinary path dependencies of your project: `squareplus_verifier = { path = "verifier" }`.
The Semaphore ingot is one more: `fe_zk_semaphore = { path = "../fe-zk/semaphore", allow_unsafe = true }`
(first recipe).

## 4. Recipes, one per proof system

The generator `fe-zk-gen` reads a verifying key, **validates it** (coordinates `< q`, points on the curve and in the
subgroup, non-degenerate, correct number of points), fingerprints it, and emits a Fe ingot: a contract that only
forwards calldata to the audited-by-test library entry point, the key as constants that the Fe compiler checks again
(`static_assert`), and a test with your real proof plus a battery of invalid ones. There is no flag to skip a check.
Exit codes: 0 valid, 1 key failed validation, 2 unreadable input. The emitted contract calls the library through one
entry point per system (`groth16::verify_calldata`, `plonk::verify_calldata`, `gnark::verify_calldata`; names in
[API.md](API.md)); for Groth16 `--lib-module FILE` emits only the key instead of a contract (first recipe). A bad key is reported and nothing is emitted:

```
$ fe-zk-gen groth16 --snarkjs bad_vk.json
error: invalid verifying key: point not on curve: alpha_g1       # exit code 1
```

Common options: `--proof FILE` and `--public FILE` (optional; without them no tests are emitted), `--name NAME`
(alphanumeric; the contract is `<NAME>Verifier` and the ingot `<name lowercase>_verifier`), `--out DIR` (without it
the key is only validated), `--fe-zk-path PATH` (written into the new `fe.toml`, relative to `--out`).

### (a) circom + snarkjs Groth16

The statement: `out = x*x + y` with `x` public and `y` secret. snarkjs orders public signals outputs first, so the
public inputs are `[out, x]`.

`my-project/circuit/square_plus.circom`:

```circom
pragma circom 2.0.0;

// out = x*x + y, with x public and y secret. Public signals: out, x.
template SquarePlus() {
    signal input x;
    signal input y;
    signal output out;
    out <== x * x + y;
}

component main {public [x]} = SquarePlus();
```

Compile, set up, prove (run in `my-project/circuit`; `ppot_0080_14.ptau` is the public Perpetual Powers of Tau file of
2^14 constraints, the pinned URL and hash are in `scripts/regen-groth16-fixtures.sh`):

```sh
curl -sSfL -o ppot_0080_14.ptau https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_14.ptau
echo "3ca1149e9349b22b0ee0649399cfb787677129b7b1189d1899fc0d615d9583db  ppot_0080_14.ptau" | sha256sum -c -

mkdir -p build                                   # circom does not create the output directory
circom square_plus.circom --r1cs --wasm -o build
echo '{"x":"3","y":"5"}' > input.json
node build/square_plus_js/generate_witness.js build/square_plus_js/square_plus.wasm input.json build/witness.wtns
snarkjs groth16 setup build/square_plus.r1cs ppot_0080_14.ptau build/0.zkey
snarkjs zkey contribute build/0.zkey build/final.zkey --name=me -e="some entropy"   # TEST ONLY, see section 7
snarkjs zkey export verificationkey build/final.zkey verification_key.json
snarkjs groth16 prove build/final.zkey build/witness.wtns proof.json public.json
snarkjs groth16 verify verification_key.json public.json proof.json                 # OK!
cat public.json                                                                      # ["14", "3"]
```

Generate the verifier (from `fe-zk/gen`; paths relative to it; the project is `../../my-project`):

```sh
cd fe-zk/gen
cargo run -q -p fe-zk-gen -- groth16 --snarkjs ../../my-project/circuit/verification_key.json \
    --proof ../../my-project/circuit/proof.json --public ../../my-project/circuit/public.json \
    --name SquarePlus --out ../../my-project/verifier --fe-zk-path ../../fe-zk
```

```
fe-zk-gen 0.2.0: groth16/bn254 key from snarkjs: VALID
  public inputs : 2
  IC points     : 3
  key digest    : 0xaef353092ee900d4a37ecfd0a1e7ef266791053734ac739609ccca3e3aaf9f9e
  emitted       : ../../my-project/verifier (ingot squareplus_verifier)
```

The digest (keccak of a documented encoding of the key, see `gen/README.md`) is in the generated header, in
`VK_DIGEST`, and returned by the contract's `vkDigest()`. The generated ingot is `fe.toml`, `src/lib.fe` (contract
`SquarePlusVerifier`) and `src/tests.fe`. Its ABI is snarkjs' `Verifier.sol`:
`verifyProof(uint256[2] a, uint256[2][2] b, uint256[2] c, uint256[2] input) returns (bool)` plus `vkDigest()`. Test it
(15 tests: the valid proof, and tampered A, B, C, public input, swapped inputs, non-canonical values, off-curve
points, a non-subgroup `B`):

```sh
cd ../../my-project/verifier && fe test     # test result: ok. 15 passed
```

Now call it from your own contract. `my-project/fe.toml` depends on `fe_zk` and on the generated ingot:

```toml
[ingot]
name = "my_project"
version = "0.1.0"

[dependencies]
fe_zk = { path = "../fe-zk", allow_unsafe = true }
squareplus_verifier = { path = "verifier" }
```

`my-project/src/lib.fe`: a contract that makes a **static call** (the verifier cannot touch your storage) and records
the output only if the proof verified:

```fe
use squareplus_verifier::SquarePlusVerifierMsg
use std::evm::{Address, Call}

msg SquarerMsg {
    #[selector = sol("claim(uint256[2],uint256[2][2],uint256[2],uint256[2])")]
    Claim { a: [u256; 2], b: [[u256; 2]; 2], c: [u256; 2], input: [u256; 2] } -> bool,
    #[selector = sol("lastOut()")]
    LastOut -> u256,
}

struct Store { verifier: Address, last_out: u256 }

pub contract Squarer {
    mut store: Store,

    init(verifier: Address) uses (mut store) { store.verifier = verifier }

    recv SquarerMsg {
        Claim { a, b, c, input } -> bool uses (mut store, call: Call) {
            // Static call: the verifier cannot touch our storage.
            let ok: bool = store
                .verifier
                .static(
                    SquarePlusVerifierMsg::VerifyProof { a: a, b: b, c: c, input: input },
                )
            if ok {
                store.last_out = input[0]
            }
            ok
        }

        LastOut -> u256 uses (store) {
            store.last_out
        }
    }
}
```

Test it like the first recipe's contract: deploy the generated verifier with `evm.create2<SquarePlusVerifier>(..)`, pass its
address to `Squarer`, and call `claim` with the proof as constants. `snarkjs zkey export soliditycalldata public.json proof.json`
prints the proof in exactly the EVM argument order (`b` already has each `Fq2` as `[imaginary, real]`); a ten-line
script turns it into `src/fixture.fe` (`make-fixture.mjs` of the first recipe does the same for Semaphore proofs).
`examples/quickstart` is this contract with its tests (valid proof accepted, tampered proof or input rejected,
nothing recorded).

Do not hand-write a verifier from a key. If the key changes, re-run the generator and commit the diff.

### (b) arkworks Groth16

`fe-zk-gen groth16 --arkworks` reads a JSON file with ark-serialize encodings (format
`arkworks-groth16-bn254-uncompressed-v1`, described in `gen/README.md`): each point is `serialize_uncompressed` of the
affine point as lowercase hex without `0x` (32-byte little-endian coordinates, flags in the top bits of the last
byte), and the public inputs are decimal strings in the order the circuit allocated them. The reference exporter for a
tiny circuit (`x*x == y`, `y` public) is `gen/fe-zk-gen/examples/arkworks_fixture.rs`; copy it into your own Cargo
project (pins: `ark-bn254`, `ark-groth16`, `ark-relations`, `ark-serialize` `=0.5.0`, `ark-snark` `=0.5.1`, `rand`
`=0.8.5`, `serde_json`) and replace the circuit, or run it as is:

```sh
cd fe-zk/gen
cargo run -q -p fe-zk-gen --example arkworks_fixture -- ../../ark-user/out     # verification_key.json, proof.json, public.json
cargo run -q -p fe-zk-gen -- groth16 --arkworks ../../ark-user/out/verification_key.json \
    --proof ../../ark-user/out/proof.json --public ../../ark-user/out/public.json \
    --name SquareArk --out ../../my-project/ark_verifier --fe-zk-path ../../fe-zk
cd ../../my-project/ark_verifier && fe test      # 14 passed
```

The generated contract has the same snarkjs ABI. The arkworks proof is not in EVM argument order; the generated
`src/tests.fe` contains it already converted (`a`, `b` with `[imaginary, real]` order, `c`), which you can copy as
fixtures. The arkworks setup above uses a seeded RNG: that is a test key; deploy a key from a real ceremony.

### (c) snarkjs PLONK

Same circuit and ptau as (a); PLONK needs no circuit-specific ceremony (the ptau is universal). Run in
`my-project/circuit`:

```sh
snarkjs plonk setup build/square_plus.r1cs ppot_0080_14.ptau build/plonk.zkey
snarkjs zkey export verificationkey build/plonk.zkey plonk_vk.json
snarkjs plonk prove build/plonk.zkey build/witness.wtns plonk_proof.json plonk_public.json
snarkjs plonk verify plonk_vk.json plonk_public.json plonk_proof.json                 # OK!
```

```sh
cd fe-zk/gen
cargo run -q -p fe-zk-gen -- plonk --snarkjs ../../my-project/circuit/plonk_vk.json \
    --proof ../../my-project/circuit/plonk_proof.json --public ../../my-project/circuit/plonk_public.json \
    --name SquarePlusPlonk --out ../../my-project/plonk_verifier --fe-zk-path ../../fe-zk
cd ../../my-project/plonk_verifier && fe test      # test result: ok
```

The contract is `SquarePlusPlonkVerifier` with the ABI of snarkjs' PLONK `Verifier.sol`:
`verifyProof(uint256[24] proof, uint256[2] input) returns (bool)` plus `vkDigest()`. Call it from your contract
exactly as in (a), with `SquarePlusPlonkVerifierMsg::VerifyProof { proof: p, input: i }`. snarkjs 0.7.6 only (the
transcript follows its equations; the key must be `protocol: plonk` on `bn128`).

### (d) gnark PLONK

The generator reads **JSON exported by the Go code in `vectors/gnark`**, not gnark's binary `vk` serialization
(fields `size`, `size_inv`, `generator`, `coset_shift`, `nb_public`, `s`, `ql qr qm qo qk`, `qcp`,
`commit_constraint_ix`, `g1`, `g2`, `g2_x`, plus `proof_words` and `public`; all words are `0x` hex). The exporter
`vectors/gnark/main.go` has a `run(name, circuit, assignment, outDir, ptauPath)` function that compiles, sets up,
proves, checks with gnark's own verifier and writes `fixture.json` and gnark's `Verifier.sol`. To use your circuit,
copy `main.go`, `ptau.go`, `go.mod`, `go.sum` into a new directory, add your circuit and replace `main()`.

The exporter builds the KZG SRS from a snarkjs `.ptau` file (a real ceremony, converted by `ptau.go`) and pins the
file's sha256. The shipped one is `ppot_0080_05` (2^5 points, circuits with a domain of at most 16). For a larger
circuit point it at a bigger PSE file: change the two constants in `ptau.go`, `PtauURL`/`PtauSHA256` (`sed` below,
same file as recipe (a): `ppot_0080_14`).

`mycircuit.go` defines your circuit (here a struct `Chain` with a secret `X` and a public `Y`, whose `Define` squares 20 times
and adds the round number, so the domain is 64) and `chainValue(x int64) *big.Int`, the same function outside the circuit
(the witness needs the public value).

```sh
mkdir gnark-export && cd gnark-export
cp ../fe-zk/vectors/gnark/{main.go,ptau.go,go.mod,go.sum} .        # plus mycircuit.go from above
sed -i '/^func main() {/,$d' main.go                                 # drop the shipped main()
cat >> main.go <<'EOF'
func main() {
	out, ptau := os.Args[1], os.Args[2]
	run("chain", &Chain{}, &Chain{X: 3, Y: chainValue(3)}, filepath.Join(out, "chain"), ptau)
}
EOF
sed -i 's#ppot_0080_05.ptau#ppot_0080_14.ptau#; s#9c1bfc4a2895b9beb1df6e95c96785dc9382bee84d84a0d8c56b1061e09d89d1#3ca1149e9349b22b0ee0649399cfb787677129b7b1189d1899fc0d615d9583db#' ptau.go
go run . out ../my-project/circuit/ppot_0080_14.ptau        # chain : gnark Go verifier accepts
```

Then generate and test (`--vk`, `--proof` and `--public` can all be the same `fixture.json`):

```sh
cd ../fe-zk/gen
F=../../gnark-export/out/chain/fixture.json
cargo run -q -p fe-zk-gen -- gnark --vk $F --proof $F --public $F \
    --name Chain --out ../../my-project/gnark_verifier --fe-zk-path ../../fe-zk
cd ../../my-project/gnark_verifier && fe test       # 17 passed
```

The contract `ChainVerifier` has the ABI of gnark's Solidity verifier,
`Verify(bytes proof, uint256[] public_inputs) returns (bool)`, plus `vkDigest()`. gnark's `MarshalSolidity()` words
concatenated are the `proof` bytes. It returns `false` for a wrong proof length or input count where gnark's own
Solidity verifier reverts. Limits: gnark v0.12.0 only; a key with an all-zero (infinity) selector commitment is
refused; PLONK with the KZG commitment scheme on BN254.

### (e) halo2-KZG

`fe-zk-gen halo2` reads a `vk.json` plus optional proof JSONs and emits a **circuit-specific, unrolled** Fe module
(gates, lookups, permutation argument and the multi-open schedule become straight-line code). The format is specified
in `vectors/halo2/SPEC.md` ("Verifying-key data").

```sh
cd fe-zk/gen
H=../vectors/halo2/standard_plonk_lookup          # shipped fixture: vk.json, shplonk.json, gwc.json
cargo run -q -p fe-zk-gen -- halo2 --vk $H/vk.json                              # validate only
cargo run -q -p fe-zk-gen -- halo2 --vk $H/vk.json --shplonk $H/shplonk.json \
    --scheme shplonk --name MyCircuit --out ../../my-project/halo2_verifier --fe-zk-path ../../fe-zk
cd ../../my-project/halo2_verifier && fe test       # 13 passed
```

`--scheme shplonk|gwc|both` chooses the multi-open scheme (default `shplonk`). One contract per scheme: `both` emits
`<NAME>ShplonkVerifier` and `<NAME>GwcVerifier`, because a single contract with both exceeds EIP-170 (SHPLONK 23.1 kB
and GWC 21.6 kB; I measured 23,137 bytes for the SHPLONK contract above). The ABI is
`verifyShplonk(uint256[57] proof, uint256[1] instances)` or `verifyGwc(uint256[61], uint256[1])`, the proof being the
raw halo2 transcript bytes as 32-byte words (the sizes depend on your circuit), plus `vkDigest()` (here a SHA-256 of
the canonical `vk.json`).

**Exporting `vk.json` for your own circuit.** halo2 has no standard JSON, and the exporter in `vectors/halo2` is
tied to its demo circuit: `src/export.rs` has `vk_json(params, pk)` (it walks `ConstraintSystem`: queries,
permutation columns, gate and lookup expression trees, fixed and sigma commitments, `transcript_repr`, the KZG
points) and `prove_shplonk` / `prove_gwc`, which write the proof with the **snark-verifier `EvmTranscript`**. The
constants `K` and `Demo::num_instance()` and the circuit type `Demo` are the demo's. For your circuit:

1. Use the same stack: `halo2-axiom` 0.5.3 (via `halo2-base` 0.5.5) and `snark-verifier` 0.2.7
   (`vectors/halo2/Cargo.toml`). Only this flavor is validated (SPEC.md is extracted from the axiom API); I did not
   test the PSE fork.
2. Copy `export.rs` and `ptau.rs`, replace `Demo`, `K` and `Demo::num_instance()` with your circuit, its `k` and its
   instance column lengths, and call `vk_json`. The `ptau.rs` converter builds a `ParamsKZG` from a snarkjs `.ptau`
   (a real ceremony, sha256 pinned); use your own SRS if you have one.
3. Prove with `EvmTranscript` (`prove_with!`) and write `{"proof": "0x...", "instances": [[...]]}` as in
   `src/bin/gen_fixtures.rs`; verify natively with halo2 first.

Supported shape (the emitter rejects anything else with an error, it never emits something partial): BN254 KZG,
**one phase and no challenges** (`num_challenges = 0`), instances not committed, SHPLONK (`ProverSHPLONK`) or GWC
(`ProverGWC`), `k` in 1..28, and distinct fixed and permutation commitments (halo2 merges byte-identical ones; the
emitter refuses a key where that happens). The README states the supported shape as "one instance column"; the
emitter flattens the instance columns into one `uint256[NI]` argument, and I exercised only the shipped
one-column circuit. The generated module is specific to the circuit; a new circuit means a new emission.
I did **not** run the exporter on a circuit of my own (it needs a full halo2 and snark-verifier build, more disk than
this machine had); the generator recipe above uses the shipped fixture.

### (f) KZG openings and EIP-4844 point evaluation

No generator: KZG checks are library calls over checked types (`G1`, `G2`, `Bn254Fr`), so an unchecked word cannot be
passed. `kzg::verify` checks one opening `f(z) = y` for a commitment `C` with a proof `pi`
(`e(C - [y]G + [z]pi, H) * e(-pi, [tau]H) = 1`, no Fiat-Shamir involved), `kzg::verify_gwc` checks `N`
polynomials at one point, `kzg::verify_shplonk` checks the fixed shape "3 polynomials at the point sets
{z1,z2}, {z1}, {z2}". The batch checks derive their challenges with fe-zk's own transcript (formulas in
`vectors/kzg/README.md`), so they interoperate with a prover that implements the same, not with an arbitrary other
implementation. `kzg::Vk::from_coords` takes whatever `[tau]_2` you supply: the SRS is your trust assumption.

`kzg_point_evaluation` wraps the EIP-4844 precompile `0x0a` with its exact 192-byte input (versioned hash, `z`, `y`,
48-byte commitment, 48-byte proof, as `Bytes48` = first 16 bytes and last 32 bytes) and checks the 64-byte answer
`(4096, BLS_MODULUS)`. It returns `false` when the precompile rejects. It needs an EVM with EIP-4844 and EIP-2537 (the
test runner has them). Like every function that reaches a precompile, both declare `uses (call: Call, ctx: Ctx)`.

`src/kzg.fe` of the ingot used in section 5 (the opening is real: arkworks 0.5.0 on the PSE `ppot_0080_05` SRS, from `vectors/kzg`; the
EIP-4844 vector is from c-kzg-4844):

```fe
use fe_zk::{Bls12381Fr, Bn254Fr, Bytes48, Field, G1, G2, kzg, kzg_point_evaluation}
use std::evm::{Call, Ctx}

// Real SRS: PSE perpetual powers of tau (ppot_0080_05); opening made with arkworks 0.5.0
// (vectors/kzg). [tau]_2 on the twist:
const TAU_X0: u256 = 18253511544609001572866960948873128266198935669250718031100637619547827597184
const TAU_X1: u256 = 10764647077472957448033591885865458661573660819003350325268673957890498500987
const TAU_Y0: u256 = 19756181390911900613508142947142748782977087973617411469215564659012323409872
const TAU_Y1: u256 = 15207030507740967976352749097256929091435606784526748170016829002013506957017
// commitment C, point z, claimed value y, opening proof pi
const C_X: u256 = 17594506662888522130950764809434494538444545583123802778088617506659113203045
const C_Y: u256 = 3969579191832173408320523733901120883812447924937432972393076507149565329564
const Z: u256 = 123456789
const Y: u256 = 5761098354726933871618415123683875487271916424664376176680479462625487858716
const PI_X: u256 = 8334224649812707509184057568585103991145220505656649813898048464844980168567
const PI_Y: u256 = 16834106107855789945255206278729981459894381212280705621623206801163918048402

#[test]
fn kzg_single_opening() uses (call: Call, ctx: Ctx) {
    // every input is a checked type: G1 / G2 / Bn254Fr cannot hold unchecked words
    let tau = G2::from_coords(TAU_X0, TAU_X1, TAU_Y0, TAU_Y1).unwrap()
    let vk = kzg::Vk::from_coords(G1::generator(), G2::generator(), tau).unwrap()
    let c = G1::from_coords(C_X, C_Y).unwrap()
    let pi = G1::from_coords(PI_X, PI_Y).unwrap()
    let z = Bn254Fr::from_canonical(Z).unwrap()
    let y = Bn254Fr::from_canonical(Y).unwrap()
    assert(kzg::verify(vk, c, z, y, pi))
    // wrong claimed value
    assert(!kzg::verify(vk, c, z, y.add(Bn254Fr::one()), pi))
}

// EIP-4844 point evaluation (precompile 0x0a). Vector from c-kzg-4844.
const VERSIONED_HASH: u256 = 0x01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b
// 48-byte compressed BLS12-381 G1 points as (first 16 bytes, last 32 bytes)
const C_HI: u256 = 0x8f59a8d2a1a625a17f3fea0fe5eb8c89
const C_LO: u256 = 0x6db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7
const PROOF_HI: u256 = 0xa62ad71d14c5719385c0686f18714304
const PROOF_LO: u256 = 0x75bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c
const BZ: u256 = 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000
const BY: u256 = 0x1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9

#[test]
fn eip4844_point_evaluation() uses (call: Call, ctx: Ctx) {
    let c = Bytes48::new(C_HI, C_LO).unwrap()
    let proof = Bytes48::new(PROOF_HI, PROOF_LO).unwrap()
    let z = Bls12381Fr::from_canonical(BZ).unwrap()
    let y = Bls12381Fr::from_canonical(BY).unwrap()
    assert(kzg_point_evaluation(VERSIONED_HASH, z, y, c, proof))
    // false (not a revert) if the precompile rejects: here a wrong claimed value
    assert(
        !kzg_point_evaluation(
            VERSIONED_HASH,
            z,
            y.add(Bls12381Fr::from_canonical(1).unwrap()),
            c,
            proof,
        ),
    )
}
```

### (g) FRI/STARK (Plonky3)

What is supported: **one family**. A Plonky3 0.4.2 `p3-uni-stark` proof of the **Fibonacci AIR** (2 columns, public
values `(a, b, x)`), over KoalaBear with the degree-4 extension, FRI with arity 2, Keccak-256 Merkle commitments and
the Keccak `SerializingChallenger32`. The verifier logic is written once; what varies is a small
configuration module (`FriParams`: trace height, blowup, final polynomial, number of queries, proof-of-work bits). Other
AIRs need new verifier code, not a new config. The shipped secure configuration (2^10 rows, blowup 16, 20 queries, 20
PoW bits) has 100 bits of security under Plonky3's conjecture, and about 59 bits (Johnson bound) or 38 bits (unique
decoding) by our own estimate of what is proven; it costs about 4.3 M gas of execution. Research-grade (see the
[checklist](#7-integration-safety-checklist)).

To regenerate the configuration module and fixture for **your own** configuration or proof, run the Plonky3 prover
crate in `vectors/plonky3-stark` and convert its output with `emit_fe.py` (Python 3, no dependencies):

```sh
cd fe-zk/vectors/plonky3-stark
cargo run --release --locked -- fib_secure --log-trace 10 --log-blowup 4 --log-final 3 --queries 20 --commit-pow 0 --query-pow 20
#   -> fib_secure/{proof,config,vectors}.json; "proof valid in Plonky3"
python3 emit_fe.py fib_secure <fixture.fe> <config.fe> secure --min-bits 100
```

`emit_fe.py <dir> <fixture .fe> <config .fe> <name> [--vectors] [--min-bits N]` writes the flat proof words (the layout is
documented at the top of `src/verifier/fri.fe`) and a module with `PARAMS`, `PROOF_WORDS` and `verify`; with
`--min-bits N` it adds `static_assert(CONJECTURED_BITS >= N)`. The emitted files `use ingot::...`, which means "inside
fe_zk". **You can keep them in your own ingot**: replace `ingot::` with `fe_zk::` and format. I did exactly this in a
separate ingot (`src/config.fe` and `src/fixture.fe`, from a fresh proof made with the commands above):

```sh
python3 -I fe-zk/vectors/plonky3-stark/emit_fe.py <dir>/fib_secure src/fixture.fe src/config.fe secure --min-bits 100
sed -i 's/ingot::/fe_zk::/g' src/config.fe src/fixture.fe && fe fmt
```

`src/tests.fe`:

```fe
use super::config::{PARAMS, PROOF_WORDS, verify}
use super::fixture::{FIB_PROOF_WORDS, FIB_PUBLIC, fib_proof}

#[test]
fn accepts_the_plonky3_proof() {
    assert(PROOF_WORDS == FIB_PROOF_WORDS)
    assert(verify(fib_proof(), FIB_PUBLIC))
}

#[test]
fn rejects_wrong_public_values() {
    assert(!verify(fib_proof(), [0, 1, 377841675]))
}
```

```sh
fe test       # rejects_wrong_public_values, accepts_the_plonky3_proof: 2 passed (about 80 s to compile)
```

As a contract, copy `examples/fri_stark`: it reads the proof from calldata into a `MemArray<u256>` instead of a
typed `[u256; 2721]` array (a typed array of that size takes minutes to compile, [section 8](#8-troubleshooting)).
Its selector is `0x12852f9a` (`verify(uint256[2721],uint256[3])`), the runtime code is 18,009 bytes (I built it:
`fe build` prints a harmless `skipping recv arm 'Verify'` warning and writes no ABI file). The calldata is the
proof words in the order of `fib_proof()` in the fixture, then the three public values. The deployed code path
was not exercised on a node here.

**Warning:** `scripts/regen-plonky3-stark.sh` at this commit does not regenerate the committed secure
configuration: it runs the prover with `--log-trace 8 --log-blowup 3 --log-final 1 --queries 27`
(101 conjectured bits), while the committed `fib_secure` and `src/verifier/fri/config_secure.fe` are
`--log-trace 10 --log-blowup 4 --log-final 3 --queries 20` (100 bits). Use the explicit command above, which
reproduces the committed `config_secure.fe` byte for byte except for the path in its header comment (checked).

## 5. Using the library directly

Everything below is in the root of the `fe_zk` ingot (`use fe_zk::Name`; the full list is [API.md](API.md)).
Snippets are files of one small ingot (dependency line as in [section 3](#3-add-fe-zk-to-your-fe-project))
and were run with `fe test` (11 tests passed).

### `Field`, `Fp<P>` and generic code

Every field is `Fp<const P: u256>`: a canonical residue in a `u256` (`Bn254Fr`, `Bn254Fq`, `KoalaBear`, `BabyBear`,
`Goldilocks`). The `Field` trait has `MODULUS`, `zero`, `one`, `is_canonical`, `from_canonical` (an `Option`; for
**untrusted** values), `reduce` (silent reduction; for constants and hash outputs only), `to_canonical`, `add`,
`sub`, `neg`, `mul`, `square`, `pow`, `inv` (an `Option`) and `eq`. Write code once over the trait:

```fe
use fe_zk::{Bn254Fr, Field, Goldilocks, KoalaBear}

/// Sum of squares over ANY fe-zk field. One body, one monomorphised copy per field.
pub fn sum_of_squares<F: Field, const N: usize>(_ xs: [F; N]) -> F {
    var acc: F = F::zero()
    var i: usize = 0
    while i < N {
        acc = acc.add(xs[i].square())
        i += 1
    }
    acc
}

/// Untrusted input: `from_canonical` returns `None` for values >= MODULUS instead of reducing.
pub fn parse<F: Field>(_ raw: u256) -> Option<F> {
    F::from_canonical(raw)
}

#[test]
fn generic_over_fields() {
    let a = sum_of_squares([Bn254Fr::reduce(3), Bn254Fr::reduce(4)])
    assert(a.to_canonical() == 25)
    let b = sum_of_squares([KoalaBear::reduce(3), KoalaBear::reduce(4)])
    assert(b.to_canonical() == 25)
    // (p - 1)^2 + 1 = 2 mod p
    let c = sum_of_squares(
        [Goldilocks::reduce(Goldilocks::MODULUS - 1), Goldilocks::one()],
    )
    assert(c.to_canonical() == 2)
}

#[test]
fn canonical_constructor_rejects_big_values() {
    assert(parse<KoalaBear>(KoalaBear::MODULUS - 1).is_some())
    assert(parse<KoalaBear>(KoalaBear::MODULUS).is_none())
    assert(Bn254Fr::from_canonical(Bn254Fr::MODULUS).is_none())
    // inverse is an Option: zero has none
    assert(Bn254Fr::zero().inv().is_none())
    let two = Bn254Fr::reduce(2)
    assert(two.mul(two.inv().unwrap()).to_canonical() == 1)
}
```

### Poseidon, Poseidon2, Merkle paths, transcript

`poseidon_1` ... `poseidon_5` are circomlib-compatible Poseidon over BN254 (checked against circomlibjs 0.1.7).
`bn254_t3_permute` and `koalabear_t16_permute` are Poseidon2; `merkle_root<H, D, N>` is one generic Merkle path over
a `Compression` trait (`Bn254Poseidon2`, `KoalaBearPoseidon2`). `Transcript` is a Keccak Fiat-Shamir hash chain:
`new(label)`, then absorb **everything** (verifying-key digest, public inputs, every commitment) in a fixed order,
then `squeeze(label)` with a distinct label per challenge.

```fe
use fe_zk::{
    Bn254Fr,
    Bn254Poseidon2,
    Field,
    KoalaBear,
    KoalaBearPoseidon2,
    Transcript,
    bn254_t3_permute,
    merkle_root,
    poseidon_2,
    poseidon_3,
}

fn fr(_ v: u256) -> Bn254Fr {
    Bn254Fr::from_canonical(v).unwrap()
}

#[test]
fn poseidon_circomlib() {
    // circomlibjs: poseidon([1, 2])
    let h = poseidon_2(fr(1), fr(2))
    assert(
        h.to_canonical()
            == 7853200120776062878684798364095072458815029376092732009249414926327459813530,
    )
    // arities 1..=5 exist: poseidon_1 .. poseidon_5
    assert(!poseidon_3(fr(1), fr(2), fr(3)).is_zero())
}

#[test]
fn poseidon2_and_merkle() {
    // BN254 Poseidon2, t = 3. The Merkle compression is perm([left, right, 0])[0].
    let s = bn254_t3_permute([fr(1), fr(2), Bn254Fr::zero()])
    // generic Merkle path: leaf, index, siblings bottom-up; None if the index does not fit the path
    let root = merkle_root<Bn254Poseidon2, Bn254Fr, 1>(fr(1), 0, [fr(2)]).unwrap()
    assert(root.eq(s[0]))
    // index 1 puts the leaf on the right
    let swapped = merkle_root<Bn254Poseidon2, Bn254Fr, 1>(fr(1), 1, [fr(2)]).unwrap()
    assert(!swapped.eq(root))
    assert(
        merkle_root<Bn254Poseidon2, Bn254Fr, 1>(fr(1), 2, [fr(2)]).is_none(),
    )

    // the same function over KoalaBear digests (8 elements), Plonky3's compression
    let z = [KoalaBear::zero(); 8]
    let kroot = merkle_root<KoalaBearPoseidon2, [KoalaBear; 8], 2>(z, 3, [z, z])
    assert(kroot.is_some())
}

#[test]
fn transcript_binds_inputs() {
    let label: u256 = 0x6d792d7a6b2d70726f746f // "my-zk-proto"
    var a = Transcript::new(label)
    a.absorb(1)
    a.absorb_point(5, 6)
    let ca = a.squeeze(1)

    var b = Transcript::new(label)
    b.absorb(2) // one absorbed word differs
    b.absorb_point(5, 6)
    let cb = b.squeeze(1)
    assert(!ca.eq(cb))

    var c = Transcript::new(label)
    c.absorb(1)
    c.absorb_point(5, 6)
    // a different squeeze label gives a different challenge
    assert(!ca.eq(c.squeeze(2)))
}
```

### BN254 `G1`, `G2`, `G2Proof` and the precompile wrappers

`G1::from_coords(x, y)` returns `None` unless both coordinates are `< q` and the point is on `y^2 = x^3 + 3`;
`(0, 0)` is the explicit point at infinity. `G2::from_coords` additionally checks the order-`r` subgroup (about
755k gas at run time; free as a `const fn` in a `static_assert`). A G2 point **taken from a proof** is a `G2Proof`
(canonical and on the twist only): it has no conversion to `G2` and can only be passed to `pairing_check_proof` /
`pairing_try_proof`, where the pairing precompile completes the check. `g1_add`, `g1_mul` and `pairing_check` wrap
the precompiles `0x06`, `0x07`, `0x08`; they check the success flag and the returned length, and a pairing is
`true` only for the word `1`. All of them declare `uses (call: Call, ctx: Ctx)`.

```fe
use fe_zk::{
    Bn254Fr,
    Field,
    G1,
    G2,
    G2Proof,
    g1_add,
    g1_mul,
    pairing_check,
    pairing_check_proof,
    pairing_try_proof,
}
use std::evm::{Call, Ctx}

#[test]
fn g1_construction() {
    // (1, 2) is the generator; (1, 3) is not on y^2 = x^3 + 3; coordinates >= q are rejected.
    assert(G1::from_coords(1, 2).unwrap().eq(G1::generator()))
    assert(G1::from_coords(1, 3).is_none())
    assert(
        G1::from_coords(
            21888242871839275222246405745257275088696311157297823662689037894645226208584,
            2,
        )
            .is_none(),
    )
    // (0, 0) is the point at infinity
    assert(G1::from_coords(0, 0).unwrap().is_infinity())
}

#[test]
fn g2_construction() {
    // G2::from_coords also checks the order-r subgroup (about 755k gas at runtime).
    let g = G2::generator()
    let again = G2::from_coords(g.x0(), g.x1(), g.y0(), g.y1()).unwrap()
    assert(again.eq(g))
    assert(
        G2::from_coords(g.x0(), g.x1(), g.y0(), g.y1() + 1).is_none(),
    )
    // A G2 point from a proof is a G2Proof: canonical + on the twist only, no subgroup check in
    // Fe (the pairing precompile does it). It can only be passed to pairing_*_proof.
    assert(
        G2Proof::from_coords(g.x0(), g.x1(), g.y0(), g.y1()).is_some(),
    )
}

#[test]
fn precompile_wrappers() uses (call: Call, ctx: Ctx) {
    let p = G1::generator()
    let two = g1_add(p, p) // precompile 0x06
    assert(two.eq(g1_mul(p, Bn254Fr::reduce(2)))) // precompile 0x07
    let q = G2::generator()
    // e(2P, Q) * e(-P, Q) * e(-P, Q) == 1, via precompile 0x08; true only if the word is exactly 1
    assert(pairing_check([two, p.neg(), p.neg()], [q, q, q]))
    assert(!pairing_check([two, p.neg()], [q, q]))
    // proof G2 operands: strict (reverts if the precompile call fails) and try (false instead)
    let qp = G2Proof::from_g2(q)
    assert(pairing_check_proof([two, p.neg(), p.neg()], [qp, qp, qp]))
    assert(pairing_try_proof([two, p.neg(), p.neg()], [qp, qp, qp]))
}
```

### BLS12-381 (EIP-2537)

`G1Bls`, `G2Bls`, `Bls12381Fr`, `bls_g1_add`, `bls_g1_mul`, `bls_g1_msm`, `bls_g2_*`, `bls_pairing_check`,
`bls_map_fp_to_g1`, `bls_map_fp2_to_g2` wrap precompiles `0x0b` to `0x11`. Untrusted points enter through
`from_words`, which lets the precompile validate them. No verifier is built on these yet beyond the KZG tests.

```fe
use fe_zk::{Bls12381Fr, Field, G1Bls, G2Bls, bls_g1_add, bls_g1_mul, bls_g2_add, bls_pairing_check}
use std::evm::{Call, Ctx}

#[test]
fn bls12_381_eip2537() uses (call: Call, ctx: Ctx) {
    // These need an EVM with the EIP-2537 precompiles (Prague or later); `fe test` has them.
    let g = G1Bls::generator()
    let h = G2Bls::generator()
    let two = bls_g1_add(g, g)
    assert(
        two.eq(bls_g1_mul(g, Bls12381Fr::from_canonical(2).unwrap())),
    )
    assert(!bls_g2_add(h, h).eq(h))
    // e(2G, H) * e(-G, H) * e(-G, H) == 1
    assert(bls_pairing_check([two, g.neg(), g.neg()], [h, h, h]))
    // untrusted words: validated by the precompile (curve + subgroup), None if rejected
    assert(G1Bls::from_words(g.words()).is_some())
    assert(G1Bls::from_words([0, 1, 0, 1]).is_none())
}
```

## 6. Compile, test, deploy

```sh
fe fmt --check          # format check (generated code is valid but not fmt-clean: run `fe fmt` on it)
fe test                 # run #[test] functions of the ingot, with a built-in EVM
fe test --filter name   # only tests whose name contains `name`
fe build                # compile every contract of the ingot into out/
```

`fe build` writes, per contract `X`: `out/X.bin` (creation code, hex text, **no `0x`**), `out/X.runtime.bin` (deployed
code) and `out/X.abi.json`. A library ingot with no contract (the `fe_zk` root) has nothing to build: build the
ingot that contains your contract and each generated verifier ingot **separately** (`fe build verifier`). Options:
`-O 0|1|2|s` (default 1), `--out-dir DIR`, `--contract NAME`.

**Code size.** The deployed code must fit EIP-170 (24,576 bytes). `scripts/code-size.sh INGOT...` builds each ingot
and prints one line per contract, exiting 1 if one is over the limit (paths relative to the `fe-zk` root):

```sh
cd fe-zk && scripts/code-size.sh ../my-project/verifier ../my-project/plonk_verifier ../my-project
# ingot                          contract                  bytes  status
# ../my-project/verifier         SquarePlusVerifier         2738  ok
# ../my-project/plonk_verifier   SquarePlusPlonkVerifier    8171  ok
# ../my-project                  Squarer                     512  ok
```

I measured: Groth16 2,738, PLONK (snarkjs) 8,171, gnark PLONK 12,597, halo2 SHPLONK 23,137 bytes, the FRI contract
18,009. The halo2 contracts are close to the limit: a larger circuit may not fit one contract.

**Deploy** with any deployer. [Section 1](#1-anonymous-membership-in-one-evening) starts `anvil` and sets `RPC` and `KEY`; for the
generated verifier and `Squarer` of recipe (a):

```sh
cd my-project && fe build && fe build verifier
VERIFIER=$(cast send --rpc-url $RPC --private-key $KEY --create "0x$(cat verifier/out/SquarePlusVerifier.bin)" --json | jq -r .contractAddress)
SQUARER=$(cast send --rpc-url $RPC --private-key $KEY --create "0x$(cat out/Squarer.bin)" "constructor(address)" $VERIFIER --json | jq -r .contractAddress)
```

`fe build` writes the creation code without `0x`, so prepend it; a constructor argument is passed as the second
and third arguments of `cast send --create`. **Check the key you deployed**: `vkDigest()` must equal the digest
that `fe-zk-gen` printed (use `bytes32` as the return type to see it in hex):

```sh
cast call --rpc-url $RPC $VERIFIER "vkDigest()(bytes32)"
# 0xaef353092ee900d4a37ecfd0a1e7ef266791053734ac739609ccca3e3aaf9f9e
```

**Call `verifyProof`** with the calldata snarkjs prints. `soliditycalldata` prints `[a],[[b]],[c],[input]`; split it
into the four `cast` arguments with `jq` (this is the Groth16 form; the PLONK output is `[proof][input]` with **no
comma** between the two arrays, so insert one with `sed 's/\]\[/],[/'` first):

```sh
CD=$(cd circuit && snarkjs zkey export soliditycalldata public.json proof.json)
mapfile -t ARGS < <(echo "[$CD]" | jq -c '.[]' | tr -d '"')
SIG="verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[2])(bool)"
cast call --rpc-url $RPC $VERIFIER "$SIG" "${ARGS[@]}"                                  # true
cast call --rpc-url $RPC $VERIFIER "$SIG" "[0x01,0x02]" "${ARGS[1]}" "${ARGS[2]}" "${ARGS[3]}"   # false
```

Your own contract, with an **explicit gas limit** (see the gas limits note below):

```sh
cast send --rpc-url $RPC --private-key $KEY $SQUARER "claim(uint256[2],uint256[2][2],uint256[2],uint256[2])" \
    "${ARGS[@]}" --gas-limit 1000000
cast call --rpc-url $RPC $SQUARER "lastOut()(uint256)"                                   # 14
```

Other families on the same anvil: PLONK
`cast call $V "verifyProof(uint256[24],uint256[2])(bool)" "${ARGS[0]}" "${ARGS[1]}"` (`true`; the same call with
`[0x0f,0x03]` as input gives `false`); gnark
`cast call $V "Verify(bytes,uint256[])(bool)" "$PROOF" "$PUB"` with
`PROOF=$(jq -r '"0x" + (.proof_words | map(.[2:]) | join(""))' fixture.json)` and `PUB=$(jq -c .public fixture.json | tr -d '"')`;
halo2
`cast call $V "verifyShplonk(uint256[57],uint256[1])(bool)" "[$(jq -r .proof shplonk.json | sed 's/^0x//' | fold -w64 | sed 's/^/0x/' | paste -sd, -)]" "$(jq -c '.instances[0]' shplonk.json | tr -d '"')"`.
All of these returned `true` for the valid proof and `false` for a changed input.

**Gas limits.** Give calls to a verifier an explicit gas limit (about 215,000 for one Groth16 check, more for the
others, see [GAS.md](GAS.md)). Since 0.2.0 every verifier reverts when the remaining gas is below what its next
precompile call needs plus a margin of 5,000 (the failure rule, [SAFETY.md](SAFETY.md) section 10), so a starved call
reverts instead of returning `false`, and gas estimation finds the right limit (tested with the first recipe's
contract: `cast send` without `--gas-limit` recorded the vote). Before 0.2.0 a starved Groth16 call returned `false`
and a contract that treated `false` as "invalid" silently did nothing; do not rely on estimation for contracts that
catch a failed inner call.

## 7. Integration safety checklist

Condensed from [SAFETY.md](SAFETY.md), [SECURITY.md](../SECURITY.md) and the README limitations. fe-zk is not audited.

- **Key provenance and the ceremony.** The generator proves a key is *well-formed*, not that it is *trustworthy*.
  Groth16 needs a circuit-specific multi-party phase 2; PLONK, halo2-KZG and KZG need an SRS nobody knows the trapdoor
  of. Every key in this repository's demos is **test only** (a single contribution with public entropy, a seeded RNG,
  or a known-tau fixture): do not deploy them, and do not copy the `snarkjs zkey contribute -e="some entropy"` line
  from recipe (a). Deploy your own ceremony's key.
- **Check `vkDigest()`.** Compare the on-chain `vkDigest()` with the digest printed by `fe-zk-gen` (and committed in the
  generated header) before you trust a deployment or point a contract at a verifier address. Another address with the
  same ABI proves nothing.
- **Groth16 proofs are malleable.** Anyone can re-randomise a valid proof into another valid proof for the same
  inputs. Never use a proof or its hash as a nullifier or a unique id: use a nullifier that is a *public input* of
  the circuit (the membership example stores `nullifierHash`, never the proof).
- **Fix the external nullifier / scope.** If a nullifier is derived from a "scope" input, the contract must pin the
  scope (the example asserts `external_nullifier == store.scope`), otherwise one member mints a fresh nullifier per
  call.
- **Bind the action to the proof.** A proof is not bound to `msg.sender`. For anything with a recipient or an amount,
  make the recipient / action hash a **public input** of the circuit and compare it with the call, or a front-runner
  submits your proof first (the proof still verifies, for the same statement).
- **`false` is not a verdict if your contract swallows reverts.** The verifiers revert, not return `false`, when the
  forwarded gas is below what a precompile call needs (the failure rule). A contract that wraps a verifier in a
  low-level call and treats any failure as "invalid proof" (for example in a slashing rule) turns a starved call into
  a verdict; forward ample gas and let the revert propagate. Within the verifier, `false` means: some input was invalid.
- **Gas caps fail closed.** Precompile calls carry hard-coded gas caps just above the current prices (EIP-1108,
  EIP-2565). A repricing makes them revert or return `false`, never accept.
- **Anonymous groups.** With `fe_zk::anon` and the Semaphore ingot: check that the proof's root is a root of *your*
  tree (`is_known_root`), pin the scope at deployment, spend the nullifier only after the proof verified, and never
  use a proof as an id. The ingot verifies a proof, not the group. The examples cap groups at 32 members, see
  [Beyond 32 members](#beyond-32-members).
- **Code size, EIP-170.** halo2 verifiers are around 22-23 kB of 24.5 kB; one contract per multi-open scheme. Check with
  `scripts/code-size.sh` after every regeneration.
- **Call verifiers with a static call and keep your own storage out of the way**: `store.verifier.static(...)`, as in
  recipe (a). The generated verifiers are pure.
- **Public input count and order are fixed by the key**; the ABI rejects a different count. snarkjs orders outputs
  before inputs. Check the order against your circuit, not against a passing test of your own making.
- **FRI/STARK security.** The shipped configuration is 100 bits only under Plonky3's conjecture on the proximity gap;
  our estimate of what is proven is about 59 bits (Johnson) or 38 (unique decoding). One AIR, one Plonky3 version.
  Do not use it where proven security is required.
- **G2 points in proofs** (Groth16 `B`) rely on the EIP-197 pairing precompile for the subgroup check. Tested on
  revm (the test EVM), not on every client. G2 points of the verifying key are checked fully, by the generator and
  again by the Fe compiler.
- **Not audited.** No external review. Report issues privately ([SECURITY.md](../SECURITY.md)).

## 8. Troubleshooting

| Symptom (exact text) | Cause and fix |
|---|---|
| `error[16-0015]: call reaches unsafe code in 'fe_zk', which is not allowed to use it` ... `= to trust 'fe_zk', add 'allow_unsafe = true' to its entry under '[dependencies]' in fe.toml` | The dependency line lacks `allow_unsafe = true`. It appears at the first call into a precompile wrapper (`g1_add`, `pairing_check`, a verifier); code that only uses `Field`/hashes compiles without it. |
| `Error: Invalid fe.toml in ingot ...: The dependency "fe_zk" has an invalid path "/abs/path"` | Fe rejects absolute dependency paths and URL schemes. Use a path relative to the `fe.toml` that contains it (`../fe-zk`). Place the projects as siblings. |
| `Error: Failed to resolve ingot dependency 'file:///.../fe-zk/': Directory does not exist` | The relative path is wrong for where the `fe.toml` lives. A generated ingot written to `--out DIR` needs `--fe-zk-path` relative to `DIR`. |
| `error[8-0081]: static assertion failed` pointing at `static_assert(g2_is_valid(BETA_X0, ...))` or `!VK_BETA.is_infinity()` in a generated `lib.fe` | A key constant was changed or corrupted: the point is off the curve or not in the subgroup, so the compiler refused the verifier. The key is bad. Regenerate from the original key; do not edit the constants. (I corrupted one digit of `BETA_Y0` to confirm: `fe build` stops in about 8 s.) |
| Long compile: 70-105 s for a tiny project | The whole `fe_zk` dependency is compiled (about 70 s on an old Xeon even for a `Field`-only test); the README measures about 20 s extra for the compile-time subgroup checks of a G2 key. There is no incremental cache to count on; keep tests in few projects, run `fe test --filter name` while iterating. |
| Compile time explodes, or a "StackOverflow" in a test, with a large `[u256; N]` | A static array of thousands of words (measured: N = 300 4 s, 600 15 s, 2721 over 4 min) makes Fe 26.5.0 slow. Use `core::ptr::MemArray<u256>` (heap, runtime length) and read calldata with `CallData::word_at`, as `examples/fri_stark` does. Copy a `MemArray` explicitly: `var r = p` aliases it. |
| `fe test src/tests/field.fe` prints `` error[2-0002]: `field` is not found `` at `use ingot::field::...` (in your own project the same error names `fe_zk`: `` `fe_zk` is not found ``) | `fe test FILE` compiles the file as a standalone module, so `ingot::`, `super::` and dependency names have nothing to resolve. Run the ingot and filter by test name instead: `cd fe-zk && fe test --filter field` (substring match; 3 tests, about 65 s). |
| `` error[2-0002]: `a` is not found `` for `var s: u256 = a` followed by a line that starts with `{` | A Fe 26.5.0 parser quirk: an identifier at the end of a line followed by a block on the next line is parsed as a struct literal `a { ... }`. Wrap it: `var s: u256 = (a)`. Repro: `scripts/probe/issue_block_after_ident.fe`; with the parentheses it passes. |
| `error: invalid verifying key: point not on curve: alpha_g1` (exit 1) from `fe-zk-gen` | The key failed validation (off curve, not in the subgroup, non-canonical coordinate, identity gamma/delta, wrong number of IC points). Nothing is emitted. Re-export the key from the prover. |
| A transaction with an estimated gas limit "succeeds" but nothing happens | Only with a contract that treats a failed inner call as `false` (and with verifiers older than 0.2.0): estimation finds the smallest gas at which nothing reverts. Pass `--gas-limit` (section 6). |
| `snarkjs zkey export soliditycalldata` output will not parse (PLONK) | The PLONK output is `[proof][input]` with no comma between the arrays; `sed 's/\]\[/],[/'` fixes it. |
| `Illegal instruction` when running `fe` | CPU without AVX; build Fe from source ([section 2](#2-install)). |
| `fe build` prints `Warning: skipping recv arm 'Verify': selector signature is unknown` | Expected for the FRI contract, which uses a raw selector; the contract is built, only the ABI file is skipped. |
| `error[...]: call reaches unsafe code ...` is not the problem, but `` `verify_proof_low` requires effect `Ctx` to be in scope `` is | Every function that reaches a precompile (all verifiers, `g1_add`, `pairing_check`, ...) declares `uses (call: Call, ctx: Ctx)` since 0.2.0 (the gas guard reads the remaining gas through `Ctx`). Add `ctx` to the `uses` list of your handler or test. |
| `node make-fixture.mjs` proves, prints the fixture, and then never exits | snarkjs leaves worker threads running. End the script with `process.exit(0)`. |
| `circom`: `invalid output path` | `-o build` needs the directory to exist: `mkdir -p build`. |

## 9. What was run, and what was not

Run with the commands as written (paths adapted only by the layout above), on a copy of this repository:

- **the first recipe**, in a scratch project: `npm install` (172 packages), `node make-fixture.mjs` (real SDK proofs
  against the ceremony keys, artifacts downloaded from `snark-artifacts.pse.dev`), `fe fmt --check && fe test`
  (2 passed, 2 m 43 s), `fe build` (2 m 35 s; `Vote.runtime.bin` 21,173 bytes), then on `anvil` 1.8.3 with `cast`:
  deploy, three `register`, `vote` (status `0x1`, gas 294,865), replay and wrong-option attempts (status `0x0`),
  `tally`; `cast send` without `--gas-limit` also recorded a vote;
- **beyond 32 members**: the `HighVerifier` and `Gate` contracts above in a scratch ingot, `fe test` with the committed
  real depth 6 proof (`vectors/semaphore/depths.json`), `fe build`;
- **the key-only verifier**: `fe-zk-gen groth16 --lib-module` on the multiplier key, `fe test` (2 passed);
- recipe (a): circom 2.2.2 compile, snarkjs 0.7.6 setup/contribute/prove/verify, `fe-zk-gen groth16 --snarkjs`,
  `fe test` of the generated ingot (15 passed) and of `my-project` (2 passed), `fe build`, `fe fmt --check`
  (run at v0.1.0; the generated contracts' ABI did not change in 0.2.0, regenerated and re-tested by `scripts/demo.sh`);
- recipes (b) to (e): as at v0.1.0, the generated ingots re-tested by `scripts/demo.sh` at 0.2.0
  (halo2, PLONK and gnark ingots; the arkworks, Go and halo2 exporters were run at v0.1.0);
- recipe (f) and section 5: all snippets as one ingot, `fe test`, on the 0.2.0 names (see the end of this list);
- recipe (g): `cargo run --release --locked` of `vectors/plonky3-stark` with the secure parameters, `emit_fe.py`,
  `fe test` of the config module in a separate ingot (2 passed), `fe build` of `examples/fri_stark` (v0.1.0);
- section 3: the git dependency against a local `file://` repository (v0.1.0);
- section 6: `anvil` and `cast` 1.8.3 as above, and at v0.1.0 deploy of the Groth16, PLONK, gnark and halo2 verifiers
  with `verifyProof`/`Verify`/`verifyShplonk` calls (true for the valid proof, false for a changed input),
  `vkDigest()`, `scripts/code-size.sh`;
- section 8: the error texts were reproduced at v0.1.0, plus the two new rows above.

**Not run** (marked here so nobody assumes otherwise):

- the `feup` installer line and the build-Fe-from-source recipe (the machine already had a source-built `fe`; the
  steps follow DESIGN.md and were not repeated);
- `scripts/setup-tools.sh` (it compiles circom from source) and the `curl` download of the ptau file (a local copy with
  the same sha256 was used);
- the Git dependency form against GitHub;
- the halo2 exporter (`vectors/halo2`) on a circuit of my own, and the Plonky3 prover on anything but the committed
  secure parameters;
- calling the FRI contract on a node, and the arkworks-generated verifier on a node;
- the Semaphore recipe against a public testnet or mainnet, and a group contract that spans a separate verifier with
  its own tree (only the verifier/gate pair above was built and tested).
