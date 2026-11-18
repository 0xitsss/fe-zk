# KZG opening fixtures

Reference fixtures for `src/verifier/kzg.fe`, produced by arkworks 0.5.0
(`ark-bn254`, `ark-ec`, `ark-ff`, `ark-poly`) and `tiny-keccak` 2.0.2 (see `Cargo.lock`).
Regenerate with `scripts/regen-kzg.sh` (downloads the pinned ptau, writes `fixtures/kzg.json`; needs a Rust
toolchain), then `scripts/gen-kzg-fe-fixtures.py`. The Fe tests in `src/tests/kzg.fe` embed the same constants.

**Real SRS.** The SRS is the PSE Perpetual Powers of Tau (challenge 0080, `ppot_0080_05.ptau`, 2^5 G1 powers;
URL and sha256 in `scripts/regen-kzg.sh`, ceremony details in `vectors/README.md` -> "Real ceremony SRS"). Nobody
knows `tau`. Commitments and opening proofs are MSMs `sum_i p_i [tau^i]G1` over the ceremony points (the
quotient is committed the same way), never computed from `tau`. `vk.tau_g2` is the ceremony's `[tau]G2`.
`cargo test --release` (here) checks the converted powers (one common tau along the chain, generator first),
MSM linearity, an honest opening and a wrong claim, and that `fixtures/kzg.json` equals a fresh regeneration.
`src/tests/kzg.fe` constants are rewritten from the JSON by `scripts/gen-kzg-fe-fixtures.py`.

## Notation

`G`, `H` = BN254 G1 / G2 generators, `r` = scalar field order, `C_i = [f_i(tau)]G`.
`vk` = `(G, H, [tau]H)`; its 10 words are `G.x, G.y, H.x0, H.x1, H.y0, H.y1, T.x0, T.x1, T.y0, T.y1`
with `T = [tau]H` and G2 coordinates in `(real, imaginary)` order. G1 infinity encodes as `(0, 0)`.

## Transcript encoding (all checks)

A challenge is `keccak256(w_0 || w_1 || ... )` reduced mod `r`, where every `w_j` is a 32-byte
big-endian word, interpreted as a 256-bit integer and reduced with `mod r` (bias about `2^-126`).
Every transcript starts with a domain tag, the ASCII string right-aligned in one word, and
binds the whole `vk`, so a challenge cannot be reused across checks or keys.

## 1. Single opening

Claim: `f(z) = y` for the polynomial committed in `C`. Proof `pi = [q(tau)]G` with `q = (f - y) / (X - z)`.

Check: `e(C - [y]G + [z]pi, H) * e(-pi, [tau]H) == 1`.
(From `f(tau) - y = (tau - z) q(tau)`.)

## 2. Batched opening at one point (GWC style)

`N >= 1` polynomials `f_i`, one point `z`, claims `f_i(z) = y_i`.

```
v = keccak256(DOM_GWC, N, vk (10 words), z, C_0.x, C_0.y, ..., C_{N-1}.x, C_{N-1}.y, y_0, ..., y_{N-1}) mod r
DOM_GWC = "FEZK-KZG-GWC-V1"
C = sum_i v^i C_i,   Y = sum_i v^i y_i,   pi = opening proof of sum_i v^i f_i at z
```

Check: single opening check of `(C, z, Y, pi)`. Word count `13 + 3N`.
This is the one-point case of Gabizon-Williamson-Ciobotaru's batching; the multi-point GWC variant
(one proof per point) is not implemented.

## 3. Multi-point opening, SHPLONK form (fixed shape)

Points `T = {z1, z2}`, `z1 != z2`. Polynomials and opening sets:

| poly | commitment | set `S_i` | claimed evaluations |
|---|---|---|---|
| `f_0` | `C_0` | `{z1, z2}` | `e00 = f_0(z1)`, `e01 = f_0(z2)` |
| `f_1` | `C_1` | `{z1}` | `e1 = f_1(z1)` |
| `f_2` | `C_2` | `{z2}` | `e2 = f_2(z2)` |

`r_i` is the interpolation polynomial of the claims of `f_i` on `S_i`
(`r_0` = the line through `(z1, e00), (z2, e01)`, `r_1 = e1`, `r_2 = e2`),
`Z_S(X) = prod_{s in S}(X - s)`.

```
y = keccak256(DOM_Y, vk (10 words), z1, z2, C_0.x, C_0.y, C_1.x, C_1.y, C_2.x, C_2.y, e00, e01, e1, e2) mod r
h(X) = sum_i y^i (f_i(X) - r_i(X)) / Z_{S_i}(X)            (exact division),   H = [h(tau)]G
u = keccak256(DOM_U, y, H.x, H.y) mod r
L(X) = sum_i y^i Z_{T \ S_i}(u) (f_i(X) - r_i(u)) - Z_T(u) h(X)               (L(u) = 0)
pi = [ (L(X) / (X - u)) (tau) ]G
DOM_Y = "FEZK-SHPLONK-Y1",  DOM_U = "FEZK-SHPLONK-U1"
```

The verifier computes `a_i = y^i Z_{T \ S_i}(u)` (`a_0 = 1`, `a_1 = y (u - z2)`, `a_2 = y^2 (u - z1)`),
`rsum = sum_i a_i r_i(u)` and

```
L = a_0 C_0 + a_1 C_1 + a_2 C_2 - [rsum] G - [Z_T(u)] H
check: e(L + [u] pi, H) * e(-pi, [tau]H) == 1
```

This is the SHPLONK (Boneh-Drake-Fisch-Gabizon, ePrint 2020/081) opening as used by halo2's
`multiopen::shplonk` (queries grouped by point set, one quotient commitment `H`, one final proof),
with the transcript replaced by the Keccak transcript above. halo2's own transcript (Blake2b/Poseidon,
different challenge order) is not reproduced; the generator for halo2 keys is a separate component.
The `u` challenge deliberately does not include `pi`, which depends on `u`.

## Fixture file

`fixtures/kzg.json`: `vk.{g1,g2,tau_g2}`, `single.{c,z,y,pi}`, `gwc.{z,cs,ys,v,y_combined,pi}`,
`shplonk.{z1,z2,cs,e00,e01,e1,e2,y,u,h,pi}`. Field elements are decimal strings; G1 is `[x, y]`,
G2 is `[x0, x1, y0, y1]`. The challenges `v`, `y`, `u` are recorded so that the Fe tests check the
Fe Keccak transcript against the Rust one.
