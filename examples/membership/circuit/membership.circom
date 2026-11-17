pragma circom 2.0.0;

include "circomlib/circuits/poseidon.circom";
include "circomlib/circuits/mux1.circom";

// Semaphore-style Poseidon Merkle membership proof (educational, simplified).
//
//   leaf          = Poseidon(secret)
//   root          = MerklePath(leaf, siblings, pathIndices)   (Poseidon(2) per level)
//   nullifierHash = Poseidon(secret, externalNullifier)
//
// Public signals, in snarkjs order (outputs first, then public inputs in declaration order):
//   [0] nullifierHash   (output)
//   [1] root            (input)
//   [2] externalNullifier (input)
//
// nullifierHash is a deterministic function of (secret, externalNullifier), so it can be
// used as a nullifier. A Groth16 proof itself is malleable and must never be a nullifier.
template Membership(depth) {
    signal input secret;
    signal input pathElements[depth];
    signal input pathIndices[depth];
    signal input root;
    signal input externalNullifier;
    signal output nullifierHash;

    component leafHash = Poseidon(1);
    leafHash.inputs[0] <== secret;

    component mux[depth];
    component hasher[depth];
    signal level[depth + 1];
    level[0] <== leafHash.out;

    for (var i = 0; i < depth; i++) {
        pathIndices[i] * (1 - pathIndices[i]) === 0;
        mux[i] = MultiMux1(2);
        mux[i].c[0][0] <== level[i];
        mux[i].c[0][1] <== pathElements[i];
        mux[i].c[1][0] <== pathElements[i];
        mux[i].c[1][1] <== level[i];
        mux[i].s <== pathIndices[i];
        hasher[i] = Poseidon(2);
        hasher[i].inputs[0] <== mux[i].out[0];
        hasher[i].inputs[1] <== mux[i].out[1];
        level[i + 1] <== hasher[i].out;
    }
    root === level[depth];

    component nh = Poseidon(2);
    nh.inputs[0] <== secret;
    nh.inputs[1] <== externalNullifier;
    nullifierHash <== nh.out;
}

component main {public [root, externalNullifier]} = Membership(10);
