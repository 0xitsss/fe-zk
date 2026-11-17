# halo2-KZG verification specification (for the Fe verifier and `fe-zk-gen`)

Executable counterpart: `src/mini_verifier.rs` (accepts the fixtures, rejects every single-byte
tamper; ~400 lines, written strictly from this document). Fixtures: `standard_plonk_lookup/`.
Reference implementation this spec was extracted from: **halo2-axiom 0.5.3** `plonk/verifier.rs`,
`plonk/{vanishing,permutation,lookup}/verifier.rs`, `poly/kzg/multiopen/{gwc,shplonk}/verifier.rs`,
and the `EvmTranscript` of **snark-verifier 0.2.7** (axiom). Cross-checked: the same proofs are also
accepted by the Solidity verifier that snark-verifier 0.2.7 generates (revm run, see section 12), so
the transcript and equations below are the ones deployed verifiers use.

Scope: BN254, KZG (`ParamsKZG<Bn256>`), one proof, **one phase, no challenges** (`num_challenges = 0`),
SHPLONK (`ProverSHPLONK`) or GWC (`ProverGWC`), instances **not committed** (`QUERY_INSTANCE = false`,
which is what both KZG multiopen verifiers use). Everything marked *static* is known from the
verifying key when the generator runs and is unrolled into constants / straight-line code.

Notation: `r` = BN254 scalar field order, `q` = base field order. `Fr` ops mod `r`. `x^n` is
`pow(x, n)`. `G` = G1 generator (`kzg.g1`, equals `g[0]` of the SRS), `H = kzg.g2`, `S = kzg.s_g2`
(= `[s]H`). `w` = `omega`, primitive `n`-th root of unity, `n = 2^k`.
All integers in transcript/proof/calldata are **32-byte big-endian**.

## 1. Verifying-key data (all in `vk.json`, all *static*)

| field | meaning |
|---|---|
| `k`, `n`, `omega`, `n_inv` (= 1/n), `delta` | domain; `delta` = `Fr::DELTA`, the coset shift of the permutation argument (copy verbatim from `delta`) |
| `cs_degree` | max constraint degree `d` |
| `blinding_factors` `bf` | = max(3, max advice rotations per column) + 2 (see `ConstraintSystem::blinding_factors`; copy verbatim) |
| `quotient_chunks` `H` | = `cs_degree - 1`: number of pieces of the quotient `h` |
| `transcript_repr` | one `Fr`: Blake2b-512("Halo2-Verify-Key", len || Debug(pinned vk)) reduced mod r. **Cannot be recomputed on chain or without halo2; the generator embeds it as a constant.** It is the only thing binding the circuit (gates, lookup expressions, fixed/sigma commitments, k, omega) to the transcript, so the generator must compute it with the same halo2 version that produced the vk. |
| `num_advice_columns` `A`, `num_fixed_columns`, `num_instance_columns`, `instance_lens` | shapes (instance lengths are fixed; wrong shape => reject) |
| `advice_queries`, `fixed_queries`, `instance_queries` | ordered lists of `(column, rotation)`. **The order of these lists is the order of evaluations in the proof (advice, fixed) and the index space used by expressions.** Order = first use while building the constraint system, so e.g. a later-created fixed column can appear earlier in `fixed_queries` (fixture: columns 6 then 5). |
| `fixed_commitments` | one G1 per fixed column (indexed by *column*, not by query) |
| `permutation.columns` | ordered list of `(type, index)` columns in the permutation argument; `chunk_len = cs_degree - 2`; `permutation.commitments` = one sigma G1 per column |
| `gates[].polys` | expression trees (ops: `const, fixed, advice, instance, neg, add, mul, scale`; leaves hold the *query index*) |
| `lookups[]` | `input_expressions[]`, `table_expressions[]` (same expression language) |
| `kzg` | `g1`, `g2`, `s_g2` |

Number of permutation sets `S = ceil(len(permutation.columns) / chunk_len)`; set `i` owns columns
`[i*chunk_len, min((i+1)*chunk_len, len))` (and the matching sigma commitments / evaluations).
Lookup count `L = len(lookups)`.

## 2. Transcript (EVM keccak, exactly `EvmTranscript`)

State: a byte buffer `buf`, initially empty.

* `absorb_scalar(s)`: append `s` as 32 bytes big-endian.
* `absorb_point(P)`: append `P.x` (32 BE) then `P.y` (32 BE). **No flags, no compression.**
* `read_scalar` / `read_point`: take the next 32 / 64 bytes of the proof, require every 32-byte
  limb canonical (`< r` for scalars, `< q` for coordinates), require the point on `y^2 = x^3 + 3`, then
  `absorb_*` it. The point at infinity `(0,0)` is **not** on the curve and is rejected; the halo2
  prover itself refuses to write infinity, so this rejection loses no honest proofs.
* `squeeze()`:
  ```
  data = buf if len(buf) != 32 else buf || 0x01      # see note
  h    = keccak256(data)
  buf  = h                                            # 32 bytes: the digest chains into the next challenge
  return int(h, big-endian) mod r                     # full 256-bit value reduced, NOT truncated
  ```
  The challenge is **not** absorbed separately: the new `buf` *is* the digest. Domain separation of
  different challenges is by position in the stream (each challenge hashes the whole history plus
  the chaining digest); there are no per-challenge labels in this transcript.
  Note (`len(buf) == 32`): happens when two squeezes follow each other with nothing absorbed
  between them (`y`/`v` in SHPLONK, and in any circuit with zero instances and zero advice the very first
  squeeze). The `0x01` byte prevents `squeeze(); squeeze()` from hashing a 32-byte buffer that could
  equal a legitimately absorbed single scalar.

Initial absorption (order matters):
1. `absorb_scalar(transcript_repr)`            -> buf = 32 bytes
2. for each instance column in order, for each value in order: `absorb_scalar(v)` (v canonical `< r`).

## 3. Proof layout and challenge schedule (order of reads / squeezes)

Counts: `A` advice cols, `Aq`/`Fq` = `len(advice_queries)`/`len(fixed_queries)`, `L` lookups, `S` perm
sets, `P` = number of perm columns, `H` quotient chunks.

| # | action | count | symbol |
|---|---|---|---|
| 1 | read points: advice commitments (column order) | `A` | `C_adv[c]` |
| 2 | **squeeze** | | `theta` |
| 3 | per lookup `j`: read `PI_j` (permuted input), `PT_j` (permuted table) | `2L` pts | |
| 4 | **squeeze** `beta`, **squeeze** `gamma` | | |
| 5 | read points: permutation product commitments `Z_i`, `i < S` | `S` | |
| 6 | per lookup `j`: read lookup product `LZ_j` | `L` pts | |
| 7 | read point: random polynomial `R` | 1 | |
| 8 | **squeeze** | | `y` |
| 9 | read points: quotient pieces `H_0..H_{H-1}` | `H` | |
| 10 | **squeeze** | | `x` |
| 11 | read scalars: `advice_evals[q]` in `advice_queries` order | `Aq` | |
| 12 | read scalars: `fixed_evals[q]` in `fixed_queries` order | `Fq` | |
| 13 | read scalar: `random_eval` | 1 | |
| 14 | read scalars: `sigma_evals[p]` in permutation-column order | `P` | |
| 15 | per perm set `i`: `z_i = Z_i(x)`, `zn_i = Z_i(wx)`, and `zl_i = Z_i(w^(-(bf+1)) x)` **only if `i < S-1`** | `3S-1` | |
| 16 | per lookup `j`: `lz = LZ_j(x)`, `lzn = LZ_j(wx)`, `la = PI_j(x)`, `lai = PI_j(w^-1 x)`, `ls = PT_j(x)` (this order) | `5L` | |
| 17 | multi-open: GWC (section 8) or SHPLONK (section 9) | | |

Instance evaluations are **not** in the proof; the verifier computes them (section 5).
Proof must be consumed exactly (no trailing bytes) and each field canonical.

Fixture (A=3, Aq=4, Fq=8, L=1, S=2, P=4, H=4): 13 points + 27 scalars + opening.
In general the proof length is
`64*(A + 2L + S + L + 1 + H) + 32*(Aq + Fq + 1 + P + (3S-1) + 5L) + OPEN`, with
`OPEN = 128` for SHPLONK (`h1`, `h2`) and `64 * m` for GWC (`m` = number of distinct points).
Fixture: SHPLONK 832 + 864 + 128 = **1824 B**; GWC (m = 4) 832 + 864 + 256 = **1952 B**.

## 4. Domain helpers

`rot(r) = w^r` for `r >= 0`, `(w^-1)^(-r)` for `r < 0` (`w^-1` = `omega^(-1)`, computed once, static).

`xn = x^n`, `Z_H(x) = xn - 1`. **Reject if `x` is a domain point** (`x - w^i = 0` for any `i` used below;
probability negligible, but inversion of 0 must not silently give 0).

Lagrange basis at `x`, valid for any integer `i` (negative allowed):
`L_i(x) = w^i * (xn - 1) * n_inv / (x - w^i)`.
Batch the inversions (Montgomery trick) if gas matters: the set of `i` needed is
`{-(bf+1) .. 0}` plus `{j - rot}` for instances.

```
l_last  = L_{-(bf+1)}(x)
l_blind = sum_{i=1..bf} L_{-i}(x)
l_0     = L_0(x)
active  = 1 - (l_last + l_blind)
```

## 5. Instance evaluations (not committed)

For every `(col, rot)` in `instance_queries` (same order, index `qi`):
`instance_evals[qi] = sum_j inst[col][j] * L_{j - rot}(x)`   (sum over the *actual* instance values;
unspecified rows are 0). Cost is `O(len(inst))` basis evaluations; consecutive `i` can share one
inversion plus a multiplication by `w` per step.

## 6. Constraint evaluation and the vanishing check

Expression evaluation `E(e)`: `const -> value`, `fixed/advice/instance -> fixed_evals/advice_evals/instance_evals[query]`,
`neg, add, mul`, `scale(a, s) = E(a) * s`. (`challenge`/`selector` do not occur: selectors are compressed into
fixed columns by keygen; multi-phase circuits are out of scope.)

Build the ordered list `cons` and fold `h_acc = fold(cons, h <- h*y + c)` starting from `0`
(i.e. the first constraint gets the highest power of `y`).
Order of `cons`:

1. **Gates:** for each gate in order, each polynomial in order: `E(poly)`.
2. **Permutation:** with `Zs[i] = (z_i, zn_i, zl_i)`:
   1. `l_0 * (1 - z_0)`
   2. `(z_{S-1}^2 - z_{S-1}) * l_last`
   3. for `i = 1..S-1`: `(z_i - zl_{i-1}) * l_0`
   4. for each set `i = 0..S-1`, with `cols_i`, `sig_i` (that set's columns and `sigma_evals`) and `cv(c)` = the
      rotation-0 evaluation of column `c` (advice/fixed/instance `*_evals[index of query (c, rotation 0)]`):
      ```
      left  = zn_i * prod_{c in cols_i} ( cv(c) + beta * sig_i[c] + gamma )
      right =  z_i * prod_{t, c in cols_i} ( cv(c) + beta * x * delta^(i*chunk_len + t) + gamma )   # t = position in set
      cons  = (left - right) * active
      ```
      (`delta^(i*chunk_len + t)` realised by starting `cur = beta*x*delta^(i*chunk_len)` and multiplying by `delta` per column.)
3. **Lookups** `j = 0..L-1` (five constraints each, this order), with `ev = (lz,lzn,la,lai,ls)`:
   `compress(exprs) = fold(exprs, a <- a*theta + E(e))` from 0;
   ```
   l_0 * (1 - lz)
   l_last * (lz^2 - lz)
   (lzn * (la + beta) * (ls + gamma)  -  lz * (compress(input_exprs) + beta) * (compress(table_exprs) + gamma)) * active
   l_0 * (la - ls)
   (la - ls) * (la - lai) * active
   ```

`expected_h = h_acc / (xn - 1)` (reject if `xn = 1`). `expected_h` is the claimed value of
`h(x) = sum_i x^{n*i} h_i(x)`; it is **not** read from the proof but enters the opening as an evaluation.
The check is therefore *folded into the multi-open*: there is no separate "vanishing equation"
comparison; a wrong `expected_h` makes the pairing fail.

Derived commitment: `HC = sum_{i<H} xn^i * H_i` (Horner from the last piece: `acc = acc*xn + H_i`).

## 7. Opening queries (order is significant; every item = `(commitment, point, eval)`)

Single proof (instances not queried):

1. for each `(col, rot)` in `advice_queries` (index `q`): `(C_adv[col], x*w^rot, advice_evals[q])`
2. for each perm set `i = 0..S-1`: `(Z_i, x, z_i)`, `(Z_i, w*x, zn_i)`
3. for `i = S-2` down to `0` (every set except the last, **in reverse**): `(Z_i, x_last, zl_i)`, `x_last = w^-(bf+1) * x`
4. for each lookup `j`: `(LZ_j, x, lz)`, `(PI_j, x, la)`, `(PT_j, x, ls)`, `(PI_j, w^-1*x, lai)`, `(LZ_j, w*x, lzn)`
5. for each `(col, rot)` in `fixed_queries` (index `q`): `(fixed_commitments[col], x*w^rot, fixed_evals[q])`
6. for each perm column `p`: `(sigma_commitments[p], x, sigma_evals[p])`
7. `(HC, x, expected_h)`, then `(R, x, random_eval)`

Identity of commitments: halo2 identifies commitments **by value**. Two different columns with
byte-identical commitments (e.g. two identical fixed columns) would be merged in the SHPLONK
grouping and GWC batch. The generator must therefore dedupe identical fixed/sigma commitments at
generation time (the verifier commitments are static) and number commitments by distinct value; the
mini verifier (indexing by column) matches halo2 only when all commitments are distinct, as in the
fixture. Advice/permutation/lookup commitments are prover-supplied: identical values there only
occur with negligible probability for an honest prover, and a malicious prover gains nothing from it
(a mismatch just fails the pairing).

The *structure* of this list (which commitment is opened at which rotation) is static; only the
evaluation values and the points `x*w^rot` depend on the proof.

## 8. GWC (`Gwc19`)

Group the query list by **point value, in order of first appearance** -> groups `g_0..g_{m-1}`
(each keeps query order). Then:

```
v      = squeeze()
W_i    = read_point()  for i in 0..m-1          # one witness per distinct point
u      = squeeze()
for each group i (pu = u^i, z_i = its point), queries (C_j, e_j) in order, pv = v^j:
    CB_i = sum_j v^j * C_j ;  EB_i = sum_j v^j * e_j
left  = sum_i u^i * W_i
right = sum_i u^i * (CB_i + z_i * W_i)  -  (sum_i u^i * EB_i) * G
accept iff  e(left, S) * e(right, -H) == 1
```

Fixture: distinct points are `x`, `w x` (shared by advice rotation 1, perm `next`, lookup `next`),
`w^-1 x`, `x_last = w^-(bf+1) x` => 4 groups, 4 witness points.

## 9. SHPLONK (`Bdfg21`)

Static structure from the query list (section 7):
* `super_points` = all distinct point values.
* For each distinct commitment (first-appearance order) its **point set**.
* **Rotation sets**: group commitments with equal point sets, ordered by first appearance of a member
  commitment; members inside a set keep first-appearance order. (Point order inside a set is irrelevant: the
  verifier only evaluates the unique interpolation polynomial; keep points and evals paired.)

Transcript: `y = squeeze()`, `v = squeeze()`, `h1 = read_point()`, `u = squeeze()`, `h2 = read_point()`.

```
for rotation set i (points T_i), members C_{i,0..}, vi = v^i:
    zdiff_i   = prod_{p in super_points \ T_i} (u - p)
    if i == 0:  z0 = prod_{p in T_0} (u - p);  z0_diff_inv = 1/zdiff_0;  zdiff_i := 1
    else:       zdiff_i := zdiff_i * z0_diff_inv                       # reject if zdiff_0 == 0
    for member j (power yj = y^j, restarting at j=0 in every set):
        r_ij(u)  = Lagrange interpolation, through (T_i, evals of C_ij at T_i), evaluated at u
        inner   += yj * C_ij ;  rin += yj * r_ij(u)
    outer  += vi * zdiff_i * inner ;  rout += vi * zdiff_i * rin
right = outer - rout*G - z0*h1 + u*h2
left  = h2
accept iff  e(left, S) * e(right, -H) == 1
```

Evaluation of `r_ij(u)` uses the points `T_i` and the member's evaluations at those points (each
`(commitment, point)` pair appears once, so the lookup is unambiguous). Interpolation can be done by
the Lagrange formula; sets are small (<= 4 points) in practice. Fixture rotation sets in order:
`S0 = {x, wx}`: [adv a, Z_1, LZ]; `S1 = {x}`: [adv b, adv c, fixed..., PT, sigma..., HC, R];
`S2 = {x, wx, x_last}`: [Z_0]; `S3 = {x, w^-1 x}`: [PI]. (Order of first appearance: adv a opens
first, `Z_0` before `Z_1` in the list but `Z_0`'s set is created when it is first seen.)

## 10. Pairing

`e(left, S) * e(right, -H) == 1` with the BN254 optimal-ate pairing (EVM precompile 0x08, input is
two `(G1, G2)` pairs; G2 coordinates in precompile order `x_im, x_re, y_im, y_re`, i.e. **`c1` before `c0`**,
opposite of the `[c0, c1]` order in `vk.json`). Check precompile success flag, returned word == 1.
`left`/`right` may legitimately be the identity only if the proof is degenerate; a precompile accepting
`(0,0)` as infinity is fine, a failing precompile call must reject.

## 11. Checks to enforce (maps to CLAUDE.md section 3)

1. canonical `Fr` for every proof scalar and every instance, canonical `Fq` for coordinates (reject, never reduce).
2. every proof point on `y^2 = x^3 + 3`; `(0,0)` rejected (see section 2).
3. G2 (`g2`, `s_g2`) are vk constants checked by the generator (on curve + subgroup); not untrusted input here.
4. instance count/lengths fixed by vk; vk-constant counts (`A, L, S, H, Aq, Fq, P`) are static, so
   the proof length is fixed -> reject any other length.
5. transcript binds vk (`transcript_repr`), all instances, all commitments, all evaluations in the order above.
   Tests: `transcript_repr` change and every proof byte flip rejected (mini verifier tests).
6. `x` not in the evaluation domain; `xn != 1`; `zdiff_0 != 0` (SHPLONK).
7. Challenge reduction is the full 256-bit digest mod r (bias 2^-126 is accepted by the reference).
8. Malleability: proofs are randomized (blinding); never use a proof or hash as a nullifier.

## 12. Reference data and baseline

* `standard_plonk_lookup/vk.json` (k=5; 3 advice, 8 fixed, 1 instance column; 2 gates, 1 lookup, S=2 perm sets, H=4).
* `shplonk.json` (1824 B proof), `gwc.json` (1952 B): produced by halo2-axiom 0.5.3 `create_proof`
  with `EvmTranscript`, each verified by halo2's own `verify_proof` before export.
* `snark_verifier_{shplonk,gwc}.sol` + `snark_verifier_gas.json`: Solidity verifier generated by snark-verifier
  0.2.7 for the same vk, deployed and called in revm with the same proof+instances: **~364.7k gas (SHPLONK), ~398.6k gas (GWC)**
  (numbers vary by a few hundred gas between regenerated proofs through calldata zero-byte count; these
  are the baseline the Fe verifiers will be compared against; pragma was relaxed from 0.8.19 to ^0.8.19 to
  compile with solc 0.8.24).
  The SRS is a test-only INSECURE seeded SRS (the files in `/data/srs/*.srs` were inspected but not used: unknown
  provenance, and k=15+ is overkill).

## 13. Port notes for the Fe verifier / generator

* The generator unrolls everything static: gate/lookup expressions into straight-line `Fr` code over
  `advice_evals[i]/fixed_evals[i]`, permutation set structure, the query list (commitment ids, rotations),
  GWC point groups / SHPLONK rotation sets and their `zdiff` products, `l_i` indices.
* Dominant EVM costs: one `ecMul` (6000) + `ecAdd` (150) per distinct opened commitment in the multi-open
  MSM, one pairing check (45000 + 2*34000 = 113k), and field inversions (modexp precompile or, better,
  one batched inversion: all denominators `x - w^i`, `xn - 1`, `zdiff_0` can share one).
* Open: Fe verifier must absorb transcript data via one `keccak256` over a memory buffer of up to
  `32 + 32*I + 64*(A+...)` bytes per squeeze; mirror snark-verifier and keep a single growing buffer, hashing
  `buf` ranges (the chaining digest sits at a fixed spot).
* Not covered by this fixture (generator must reject or extend): multiple phases / challenges
  (`num_challenges > 0`), instance rotations other than 0 (formula in section 5 already general, untested),
  multiple instance columns (general, untested), more than one proof per verification, committed instances
  (`QUERY_INSTANCE = true`, IPA-style).
