# Using fe-zk

A practical guide for a Fe developer who either has a circuit and a proof system and wants an on-chain verifier in Fe,
or wants to use the field, curve and hash layer directly. Everything here was run, not just written: each recipe was
executed end to end on a copy of the repository, and every Fe and Rust snippet below is a file that compiled
and passed `fe test` or `cargo run`. The last section ("What was run, and what was not") lists the exceptions.

Tested with: `fe 26.5.0 (3dab95f)`, `fe-zk-gen 0.2.0`, circom 2.2.2, snarkjs 0.7.6, gnark v0.12.0 (Go),
arkworks 0.5.0, Plonky3 0.4.2, Foundry (`anvil`, `cast`) 1.8.3.

fe-zk has **not been audited**; read [Integration safety checklist](#6-integration-safety-checklist) before you
deploy anything, and [SECURITY.md](../SECURITY.md) for the threat model.

All commands assume this layout (the repository and your project are siblings, so that the relative paths in
`fe.toml` work, see [section 2](#2-add-fe-zk-to-your-fe-project)):

```
work/
  fe-zk/        git clone of this repository
  my-project/   your Fe project
```

Compile times are long. On the machine used for this guide (an old Xeon without AVX), `fe test` or `fe build` of a
project that depends on `fe_zk` takes 70 to 105 seconds, even for a test that only uses `Field`. Plan for it.

## Contents

1. [Install](#1-install)
2. [Add fe-zk to your Fe project](#2-add-fe-zk-to-your-fe-project)
3. [Recipes, one per proof system](#3-recipes-one-per-proof-system)
   (a) [circom + snarkjs Groth16](#a-circom--snarkjs-groth16) (b) [arkworks Groth16](#b-arkworks-groth16)
   (c) [snarkjs PLONK](#c-snarkjs-plonk) (d) [gnark PLONK](#d-gnark-plonk) (e) [halo2-KZG](#e-halo2-kzg)
   (f) [KZG openings and EIP-4844](#f-kzg-openings-and-eip-4844-point-evaluation) (g) [FRI/STARK](#g-fristark-plonky3)
4. [Using the library directly](#4-using-the-library-directly)
5. [Compile, test, deploy](#5-compile-test-deploy)
6. [Integration safety checklist](#6-integration-safety-checklist)
7. [Troubleshooting](#7-troubleshooting)
8. [What was run, and what was not](#8-what-was-run-and-what-was-not)

## 1. Install

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
| Foundry (`anvil`, `cast`), `jq` | 1.8.3 | deploying and calling, section 5 |

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

## 2. Add fe-zk to your Fe project

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
  ([Troubleshooting](#7-troubleshooting)). It trusts the whole `fe_zk` ingot, so pin what you trust.
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

## 3. Recipes, one per proof system

The generator `fe-zk-gen` reads a verifying key, **validates it** (coordinates `< q`, points on the curve and in the
subgroup, non-degenerate, correct number of points), fingerprints it, and emits a Fe ingot: a contract that only
forwards calldata to the audited-by-test library entry point, the key as constants that the Fe compiler checks again
(`static_assert`), and a test with your real proof plus a battery of invalid ones. There is no flag to skip a check.
Exit codes: 0 valid, 1 key failed validation, 2 unreadable input. A bad key is reported and nothing is emitted:

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
snarkjs zkey contribute build/0.zkey build/final.zkey --name=me -e="some entropy"   # TEST ONLY, see section 6
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

For the test you need the proof as Fe constants. `snarkjs zkey export soliditycalldata` prints the proof in exactly
the EVM argument order (`b` already has each `Fq2` as `[imaginary, real]`). This small script turns that output
into `src/fixture.fe` (`my-project/make-fixture.mjs`):

```js
// usage: snarkjs zkey export soliditycalldata public.json proof.json | node make-fixture.mjs > src/fixture.fe
import { readFileSync } from "node:fs";
const [a, b, c, input] = JSON.parse("[" + readFileSync(0, "utf8") + "]");
const arr = (xs) => "[" + xs.join(", ") + "]";
console.log(`pub const A: [u256; 2] = ${arr(a)}`);
console.log(`pub const B: [[u256; 2]; 2] = [${b.map(arr).join(", ")}]`);
console.log(`pub const C: [u256; 2] = ${arr(c)}`);
console.log(`pub const INPUT: [u256; 2] = ${arr(input)}`);
```

```sh
cd my-project
(cd circuit && snarkjs zkey export soliditycalldata public.json proof.json) | node make-fixture.mjs > src/fixture.fe
fe fmt
```

`my-project/src/tests.fe` (files under `src/` are modules; `use super::...` reaches `lib.fe`):

```fe
use super::{Squarer, SquarerMsg}
use super::fixture::{A, B, C, INPUT}
use squareplus_verifier::SquarePlusVerifier
use std::evm::{Address, Evm}

fn deploy() -> Address uses (evm: mut Evm) {
    let v = evm.create2<SquarePlusVerifier>(value: Wei::zero(), args: (), salt: 0)
    evm.create2<Squarer>(value: Wei::zero(), args: (v,), salt: 1)
}

fn claim(_ s: Address, _ a: [u256; 2], _ input: [u256; 2]) -> bool
uses (evm: mut Evm)
{
    let ok: bool = evm
        .call(
            addr: s,
            gas: 10000000,
            value: Wei::zero(),
            message: SquarerMsg::Claim { a: a, b: B, c: C, input: input },
        )
    ok
}

#[test]
fn valid_proof_is_recorded() uses (evm: mut Evm) {
    let s = deploy()
    assert(claim(s, A, INPUT))
    let last: u256 = evm
        .call(
            addr: s,
            gas: 10000000,
            value: Wei::zero(),
            message: SquarerMsg::LastOut {},
        )
    assert(last == INPUT[0])
}

#[test]
fn tampered_proof_or_input_is_rejected() uses (evm: mut Evm) {
    let s = deploy()
    assert(!claim(s, [A[0] + 1, A[1]], INPUT))
    assert(!claim(s, A, [INPUT[0] + 1, INPUT[1]]))
    let last: u256 = evm
        .call(
            addr: s,
            gas: 10000000,
            value: Wei::zero(),
            message: SquarerMsg::LastOut {},
        )
    assert(last == 0)
}
```

```sh
fe fmt --check && fe test        # tampered_proof_or_input_is_rejected, valid_proof_is_recorded: 2 passed
```

Do not hand-write a verifier from a key. If the key changes, re-run the generator and commit the diff.

### (b) arkworks Groth16

`fe-zk-gen groth16 --arkworks` reads a JSON file with ark-serialize encodings (format
`arkworks-groth16-bn254-uncompressed-v1`, described in `gen/README.md`): each point is `serialize_uncompressed` of the
affine point as lowercase hex without `0x` (32-byte little-endian coordinates, flags in the top bits of the last
byte), and the public inputs are decimal strings in the order the circuit allocated them. The full reference
exporter is `gen/fe-zk-gen/examples/arkworks_fixture.rs`
(`cargo run -p fe-zk-gen --example arkworks_fixture -- <dir>`). The minimal version for your own circuit is a Cargo
project (`Cargo.toml` pins what `fe-zk-gen` uses):

```toml
[package]
name = "ark-export"
version = "0.1.0"
edition = "2021"

[dependencies]
ark-bn254 = { version = "=0.5.0", default-features = false, features = ["curve"] }
ark-groth16 = { version = "=0.5.0", default-features = false }
ark-relations = "=0.5.0"
ark-serialize = "=0.5.0"
ark-snark = "=0.5.1"
rand = "=0.8.5"
serde_json = "1"
```

`src/main.rs` (circuit: `x*x == y`, `y` public):

```rust
use ark_bn254::{Bn254, Fr};
use ark_groth16::Groth16;
use ark_relations::{
    lc,
    r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError},
};
use ark_serialize::CanonicalSerialize;
use ark_snark::SNARK;
use rand::{rngs::StdRng, SeedableRng};
use serde_json::json;

const FORMAT: &str = "arkworks-groth16-bn254-uncompressed-v1";

/// x * x == y, with y public and x secret.
#[derive(Clone)]
struct Square {
    x: Option<Fr>,
}

impl ConstraintSynthesizer<Fr> for Square {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let xv = self.x;
        let y = cs.new_input_variable(|| {
            let v = xv.ok_or(SynthesisError::AssignmentMissing)?;
            Ok(v * v)
        })?;
        let x = cs.new_witness_variable(|| xv.ok_or(SynthesisError::AssignmentMissing))?;
        cs.enforce_constraint(lc!() + x, lc!() + x, lc!() + y)?;
        Ok(())
    }
}

/// ark-serialize `serialize_uncompressed`, lowercase hex without 0x: the format fe-zk-gen reads.
fn hx<T: CanonicalSerialize>(t: &T) -> String {
    let mut v = Vec::new();
    t.serialize_uncompressed(&mut v).unwrap();
    v.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: ark-export <out dir>");
    std::fs::create_dir_all(&out).unwrap();
    // TEST ONLY: a seeded RNG makes the toxic waste reproducible. Use your real ceremony's key.
    let mut rng = StdRng::seed_from_u64(1);
    let (pk, vk) =
        Groth16::<Bn254>::circuit_specific_setup(Square { x: None }, &mut rng).unwrap();
    let x = Fr::from(7u64);
    let proof = Groth16::<Bn254>::prove(&pk, Square { x: Some(x) }, &mut rng).unwrap();
    let public = vec![x * x];
    assert!(Groth16::<Bn254>::verify(&vk, &public, &proof).unwrap());

    let vk_json = json!({
        "format": FORMAT,
        "n_public": public.len(),
        "alpha_g1": hx(&vk.alpha_g1),
        "beta_g2": hx(&vk.beta_g2),
        "gamma_g2": hx(&vk.gamma_g2),
        "delta_g2": hx(&vk.delta_g2),
        "gamma_abc_g1": vk.gamma_abc_g1.iter().map(hx).collect::<Vec<_>>(),
    });
    let proof_json = json!({
        "format": FORMAT, "a": hx(&proof.a), "b": hx(&proof.b), "c": hx(&proof.c),
    });
    // public inputs: decimal strings, in the order the circuit allocated them
    let public_json: Vec<String> = public.iter().map(|p| p.to_string()).collect();
    let w = |name: &str, v: serde_json::Value| {
        std::fs::write(format!("{out}/{name}"), serde_json::to_string_pretty(&v).unwrap()).unwrap()
    };
    w("verification_key.json", vk_json);
    w("proof.json", proof_json);
    w("public.json", json!(public_json));
}
```

```sh
cargo run -q -- out                      # writes out/verification_key.json, proof.json, public.json
cd fe-zk/gen
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

`mycircuit.go` (20 squarings, so the domain is 64):

```go
package main

import (
	"math/big"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/frontend"
)

const rounds = 20

// Chain: acc = x; 20 times acc = acc*acc + i; public y = acc. x is secret.
type Chain struct {
	X frontend.Variable
	Y frontend.Variable `gnark:",public"`
}

func (c *Chain) Define(api frontend.API) error {
	acc := c.X
	for i := 0; i < rounds; i++ {
		acc = api.Add(api.Mul(acc, acc), i)
	}
	api.AssertIsEqual(acc, c.Y)
	return nil
}

// chainValue computes the same function outside the circuit (the witness needs the public value).
func chainValue(x int64) *big.Int {
	r := ecc.BN254.ScalarField()
	acc := big.NewInt(x)
	for i := 0; i < rounds; i++ {
		acc.Mul(acc, acc).Add(acc, big.NewInt(int64(i))).Mod(acc, r)
	}
	return acc
}
```

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
passed. `kzg_verify` checks one opening `f(z) = y` for a commitment `C` with a proof `pi`
(`e(C - [y]G + [z]pi, H) * e(-pi, [tau]H) = 1`, no Fiat-Shamir involved), `kzg_batch_verify_gwc` checks `N`
polynomials at one point, `kzg_verify_shplonk` checks the fixed shape "3 polynomials at the point sets
{z1,z2}, {z1}, {z2}". The batch checks derive their challenges with fe-zk's own transcript (formulas in
`vectors/kzg/README.md`), so they interoperate with a prover that implements the same, not with an arbitrary other
implementation. `KzgVk::new` takes whatever `[tau]_2` you supply: the SRS is your trust assumption.

`kzg_point_evaluation` wraps the EIP-4844 precompile `0x0a` with its exact 192-byte input (versioned hash, `z`, `y`,
48-byte commitment, 48-byte proof, as `Bytes48` = first 16 bytes and last 32 bytes) and checks the 64-byte answer
`(4096, BLS_MODULUS)`. It returns `false` when the precompile rejects. It needs an EVM with EIP-4844 and EIP-2537 (the
test runner has them).

`src/kzg.fe` of the ingot used in section 4 (the opening is real: arkworks 0.5.0 on the PSE `ppot_0080_05` SRS, from `vectors/kzg`; the
EIP-4844 vector is from c-kzg-4844):

```fe
use fe_zk::{Bls12381Fr, Bn254Fr, Bytes48, Field, G1, G2, KzgVk, kzg_point_evaluation, kzg_verify}
use std::evm::Call

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
fn kzg_single_opening() uses (call: Call) {
    // every input is a checked type: G1 / G2 / Bn254Fr cannot hold unchecked words
    let tau = G2::from_coords(TAU_X0, TAU_X1, TAU_Y0, TAU_Y1).unwrap()
    let vk = KzgVk::new(G1::generator(), G2::generator(), tau).unwrap()
    let c = G1::from_coords(C_X, C_Y).unwrap()
    let pi = G1::from_coords(PI_X, PI_Y).unwrap()
    let z = Bn254Fr::from_canonical(Z).unwrap()
    let y = Bn254Fr::from_canonical(Y).unwrap()
    assert(kzg_verify(vk, c, z, y, pi))
    // wrong claimed value
    assert(!kzg_verify(vk, c, z, y.add(Bn254Fr::one()), pi))
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
fn eip4844_point_evaluation() uses (call: Call) {
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
[checklist](#6-integration-safety-checklist)).

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
typed `[u256; 2721]` array (a typed array of that size takes minutes to compile, [section 7](#7-troubleshooting)).
Its selector is `0x12852f9a` (`verify(uint256[2721],uint256[3])`), the runtime code is 18,009 bytes (I built it:
`fe build` prints a harmless `skipping recv arm 'Verify'` warning and writes no ABI file). The calldata is the
proof words in the order of `fib_proof()` in the fixture, then the three public values. The deployed code path
was not exercised on a node here.

**Warning:** `scripts/regen-plonky3-stark.sh` at this commit does not regenerate the committed secure
configuration: it runs the prover with `--log-trace 8 --log-blowup 3 --log-final 1 --queries 27`
(101 conjectured bits), while the committed `fib_secure` and `src/verifier/fri/config_secure.fe` are
`--log-trace 10 --log-blowup 4 --log-final 3 --queries 20` (100 bits). Use the explicit command above, which
reproduces the committed `config_secure.fe` byte for byte except for the path in its header comment (checked).

## 4. Using the library directly

Everything below is in the root of the `fe_zk` ingot (`use fe_zk::Name`; the full list is [API.md](API.md)).
Snippets are files of one small ingot (dependency line as in [section 2](#2-add-fe-zk-to-your-fe-project))
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
`true` only for the word `1`. All of them declare `uses (call: Call)`.

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
use std::evm::Call

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
fn precompile_wrappers() uses (call: Call) {
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
use std::evm::Call

#[test]
fn bls12_381_eip2537() uses (call: Call) {
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

## 5. Compile, test, deploy

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

**Deploy** with any deployer. With Foundry, against a local `anvil` (I used port 18545; `anvil` defaults to 8545):

```sh
anvil --port 18545 &
export RPC=http://127.0.0.1:18545
export KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80    # anvil's public dev key #0

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

Your own contract, with an **explicit gas limit** (see the warning below):

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

**Warning: gas estimation.** `cast send` without `--gas-limit` estimated 63,146 gas for `claim`, the transaction
succeeded with status `0x1`, and **nothing was recorded**: `lastOut()` stayed `0`. The estimator found the smallest gas
at which the transaction does not revert; with less than the verification needs (about 200k), the inner static call
runs out of gas, returns `false`, and `claim` completes normally without writing. With `--gas-limit 1000000` it
recorded `14`. A contract that treats `false` as "invalid" cannot tell an invalid proof from a starved call. Set gas
limits explicitly (and see the checklist).

## 6. Integration safety checklist

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
- **`false` is not a verdict.** `pairing_try_proof` (used by the Groth16 path) returns `false` for *any* failed
  precompile call, including one that fails because too little gas was forwarded (see the gas-estimation warning in
  section 5). Do not use `false` as evidence of an invalid proof, for example in a slashing rule. Forward ample gas and
  treat `false` as "not accepted now".
- **Gas caps fail closed.** Precompile calls carry hard-coded gas caps just above the current prices (EIP-1108,
  EIP-2565). A repricing makes them revert or return `false`, never accept.
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

## 7. Troubleshooting

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
| A transaction with an estimated gas limit "succeeds" but nothing happens | Gas estimation under-estimates a contract that treats a failed static call as `false`. Pass `--gas-limit` (section 5). |
| `snarkjs zkey export soliditycalldata` output will not parse (PLONK) | The PLONK output is `[proof][input]` with no comma between the arrays; `sed 's/\]\[/],[/'` fixes it. |
| `Illegal instruction` when running `fe` | CPU without AVX; build Fe from source ([section 1](#1-install)). |
| `fe build` prints `Warning: skipping recv arm 'Verify': selector signature is unknown` | Expected for the FRI contract, which uses a raw selector; the contract is built, only the ABI file is skipped. |
| `circom`: `invalid output path` | `-o build` needs the directory to exist: `mkdir -p build`. |

## 8. What was run, and what was not

Run on a copy of this repository, with the commands as written (paths adapted only by the layout above):

- recipe (a): circom 2.2.2 compile, snarkjs 0.7.6 setup/contribute/prove/verify, `fe-zk-gen groth16 --snarkjs`,
  `fe test` of the generated ingot (15 passed) and of `my-project` (2 passed), `fe build`, `fe fmt --check`;
- recipe (b): the Rust exporter above, `fe-zk-gen groth16 --arkworks`, `fe test` (14 passed);
- recipe (c): snarkjs PLONK, `fe-zk-gen plonk`, `fe test`, `fe build`;
- recipe (d): the Go exporter with the circuit above (`go run`), `fe-zk-gen gnark`, `fe test` (17 passed), `fe build`;
- recipe (e): `fe-zk-gen halo2` (`--scheme shplonk`, `gwc`, `both`, and validate-only), `fe test` (13 passed) and
  `fe build` of the SHPLONK ingot;
- recipe (f) and section 4: all snippets, as one ingot, `fe test` (11 passed);
- recipe (g): `cargo run --release --locked` of `vectors/plonky3-stark` with the secure parameters, `emit_fe.py`,
  `fe test` of the config module in a separate ingot (2 passed), `fe build` of `examples/fri_stark`;
- section 2: the git dependency against a local `file://` repository;
- section 5: `anvil` 1.8.3 and `cast` 1.8.3: deploy of the Groth16, PLONK, gnark and halo2 verifiers and of `Squarer`,
  `verifyProof`/`Verify`/`verifyShplonk` calls (true for the valid proof, false for a changed input), `vkDigest()`,
  `claim`, `scripts/code-size.sh`;
- section 7: every error text was reproduced, except `Illegal instruction`.

**Not run** (marked here so nobody assumes otherwise):

- the `feup` installer line and the build-Fe-from-source recipe (the machine already had a source-built `fe`; the
  steps follow DESIGN.md and were not repeated);
- `scripts/setup-tools.sh` (it compiles circom from source; a circom 2.2.2 binary and the pinned snarkjs were
  already present) and the `curl` download of the ptau file (a local copy with the same sha256 was used);
- the Git dependency form against GitHub;
- the halo2 exporter (`vectors/halo2`) on a circuit of my own, and the Plonky3 prover on anything but the committed
  secure parameters;
- calling the FRI contract on a node, and the arkworks-generated verifier on a node.
