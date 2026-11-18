package main

// snarkjs .ptau (Perpetual Powers of Tau, PSE ppot_0080_NN) -> gnark-crypto bn254 KZG SRS.
//
// ptau layout (snarkjs binfileutils): magic "ptau", version u32, nSections u32, then per section
// `id u32 | size u64 | data` (little endian). Section 1 = header (n8 u32, q[n8], power u32,
// ceremonyPower u32), section 2 = tauG1 (2^(power+1)-1 points [tau^i]G1), section 3 = tauG2
// (2^power points [tau^i]G2). Points are uncompressed affine, every Fq coordinate is 32 bytes
// little endian in Montgomery form (x * 2^256 mod q). gnark-crypto's fp.Element is also
// Montgomery with R = 2^256 and 4 little-endian uint64 limbs, so the bytes map to limbs 1:1.
// Every coordinate is checked < q, every point on curve and in the subgroup.

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"math/big"
	"os"

	"github.com/consensys/gnark-crypto/ecc/bn254"
	"github.com/consensys/gnark-crypto/ecc/bn254/fp"
	"github.com/consensys/gnark-crypto/ecc/bn254/kzg"
)

// Pinned ceremony file: PSE perpetual powers of tau, challenge 0080, power 5 (32 G1 powers are
// enough for domain size n <= 16: the canonical SRS needs n+3 points).
const (
	PtauURL    = "https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_05.ptau"
	PtauSHA256 = "9c1bfc4a2895b9beb1df6e95c96785dc9382bee84d84a0d8c56b1061e09d89d1"
)

func fpFromMont(b []byte) fp.Element {
	var e fp.Element
	var be [32]byte
	for i := 0; i < 4; i++ {
		e[i] = binary.LittleEndian.Uint64(b[8*i:])
		binary.BigEndian.PutUint64(be[8*(3-i):], e[i])
	}
	var v big.Int
	v.SetBytes(be[:])
	if v.Cmp(fp.Modulus()) >= 0 {
		panic("non-canonical ptau coordinate")
	}
	return e
}

type ptau struct {
	power int
	g1    []bn254.G1Affine
	g2    []bn254.G2Affine
}

func readPtau(path string, maxG1, maxG2 int) ptau {
	raw, err := os.ReadFile(path)
	must(err)
	sum := sha256.Sum256(raw)
	if got := hex.EncodeToString(sum[:]); got != PtauSHA256 {
		panic(fmt.Sprintf("ptau sha256 mismatch: got %s want %s", got, PtauSHA256))
	}
	if !bytes.Equal(raw[:4], []byte("ptau")) {
		panic("not a ptau file")
	}
	le32 := func(o int) int { return int(binary.LittleEndian.Uint32(raw[o:])) }
	nsec := le32(8)
	off := 12
	secs := map[int][2]int{}
	for i := 0; i < nsec; i++ {
		id := le32(off)
		sz := int(binary.LittleEndian.Uint64(raw[off+4:]))
		secs[id] = [2]int{off + 12, sz}
		off += 12 + sz
	}
	if off != len(raw) {
		panic("trailing bytes / truncated ptau")
	}
	h := secs[1][0]
	if le32(h) != 32 {
		panic("n8 != 32")
	}
	power := le32(h + 4 + 32)
	s2, sz2 := secs[2][0], secs[2][1]
	s3, sz3 := secs[3][0], secs[3][1]
	if sz2 != ((1<<(power+1))-1)*64 || sz3 != (1<<power)*128 {
		panic("unexpected ptau section sizes")
	}
	n1, n2 := min(maxG1, sz2/64), min(maxG2, sz3/128)
	out := ptau{power: power}
	for i := 0; i < n1; i++ {
		o := s2 + 64*i
		var p bn254.G1Affine
		p.X = fpFromMont(raw[o : o+32])
		p.Y = fpFromMont(raw[o+32 : o+64])
		if !p.IsOnCurve() || !p.IsInSubGroup() {
			panic("tauG1 point invalid")
		}
		out.g1 = append(out.g1, p)
	}
	for i := 0; i < n2; i++ {
		o := s3 + 128*i
		var p bn254.G2Affine
		p.X.A0 = fpFromMont(raw[o : o+32])
		p.X.A1 = fpFromMont(raw[o+32 : o+64])
		p.Y.A0 = fpFromMont(raw[o+64 : o+96])
		p.Y.A1 = fpFromMont(raw[o+96 : o+128])
		if !p.IsOnCurve() || !p.IsInSubGroup() {
			panic("tauG2 point invalid")
		}
		out.g2 = append(out.g2, p)
	}
	return out
}

// srsFromPtau builds gnark's canonical SRS (n+3 powers) and Lagrange SRS (n points) for domain
// size n, the shapes plonk.Setup expects (see gnark test/unsafekzg).
// Consistency checks: tauG1[0] and tauG2[0] are the generators, every consecutive pair of G1
// powers has the ratio of the G2 pair (e(P_i, [tau]G2) = e(P_{i+1}, G2)), and the Lagrange basis
// commitments sum to the generator (sum of L_i = 1).
func srsFromPtau(path string, n uint64) (canonical, lagrange *kzg.SRS) {
	size := int(n) + 3
	p := readPtau(path, size, 2)
	if len(p.g1) != size || len(p.g2) != 2 {
		panic("ptau too small for the domain")
	}
	_, _, g1, g2 := bn254.Generators()
	if p.g1[0] != g1 || p.g2[0] != g2 {
		panic("tauG1[0]/tauG2[0] must be the generators")
	}
	for i := 0; i+1 < len(p.g1); i++ {
		var negNext bn254.G1Affine
		negNext.Neg(&p.g1[i+1])
		ok, err := bn254.PairingCheck([]bn254.G1Affine{p.g1[i], negNext}, []bn254.G2Affine{p.g2[1], g2})
		must(err)
		if !ok {
			panic(fmt.Sprintf("G1 powers %d,%d inconsistent with [tau]G2", i, i+1))
		}
	}
	canonical = &kzg.SRS{}
	canonical.Pk.G1 = p.g1
	canonical.Vk.G1 = g1
	canonical.Vk.G2 = [2]bn254.G2Affine{p.g2[0], p.g2[1]}
	canonical.Vk.Lines[0] = bn254.PrecomputeLines(canonical.Vk.G2[0])
	canonical.Vk.Lines[1] = bn254.PrecomputeLines(canonical.Vk.G2[1])

	lag, err := kzg.ToLagrangeG1(p.g1[:n])
	must(err)
	lagrange = &kzg.SRS{Vk: canonical.Vk}
	lagrange.Pk.G1 = lag
	var sum bn254.G1Jac
	for i := range lag {
		var j bn254.G1Jac
		j.FromAffine(&lag[i])
		sum.AddAssign(&j)
	}
	var sa bn254.G1Affine
	sa.FromJacobian(&sum)
	if sa != g1 {
		panic("Lagrange SRS does not sum to the generator")
	}
	return
}
