// Fixture generator for the fe-zk gnark PLONK verifier.
//
// Compiles two tiny circuits with gnark v0.12.0 (PLONK / BN254 / KZG), proves them, verifies
// them with gnark's own Go verifier, and exports:
//   - gnark's Solidity verifier (vk.ExportSolidity) as <name>/Verifier.sol
//   - <name>/fixture.json: vk words, proof words (gnark MarshalSolidity layout, 32-byte
//     words), public inputs and the Fiat-Shamir challenges recomputed independently here.
//
// The KZG SRS is the real Perpetual Powers of Tau ceremony (PSE ppot_0080_05, sha256 pinned in
// ptau.go; nobody knows tau), converted to gnark's canonical + Lagrange form by srsFromPtau.
// Usage: go run . <out dir> <path to ppot_0080_05.ptau>
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"path/filepath"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark-crypto/ecc/bn254"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr/hash_to_field"
	"github.com/consensys/gnark-crypto/ecc/bn254/kzg"
	fiatshamir "github.com/consensys/gnark-crypto/fiat-shamir"
	"github.com/consensys/gnark/backend/plonk"
	plonkbn254 "github.com/consensys/gnark/backend/plonk/bn254"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/scs"
)

// Mul: x * y + 7 == z with public x, z and secret y.
type Mul struct {
	X frontend.Variable `gnark:",public"`
	Y frontend.Variable
	Z frontend.Variable `gnark:",public"`
}

func (c *Mul) Define(api frontend.API) error {
	api.AssertIsEqual(api.Add(api.Mul(c.X, c.Y), 7), c.Z)
	return nil
}

// MulCommit is Mul plus one BSB22 commitment (api.Commit) that is used in a constraint.
type MulCommit struct {
	X frontend.Variable `gnark:",public"`
	Y frontend.Variable
	Z frontend.Variable `gnark:",public"`
}

func (c *MulCommit) Define(api frontend.API) error {
	api.AssertIsEqual(api.Add(api.Mul(c.X, c.Y), 7), c.Z)
	committer, ok := api.(frontend.Committer)
	if !ok {
		return fmt.Errorf("no committer")
	}
	cm, err := committer.Commit(c.X, c.Y)
	if err != nil {
		return err
	}
	api.AssertIsDifferent(cm, 0)
	return nil
}

func w32(b []byte) string { return "0x" + hex.EncodeToString(b) }

func frHex(e *fr.Element) string { b := e.Bytes(); return w32(b[:]) }

func g1Words(p *bn254.G1Affine) []string {
	b := p.RawBytes()
	return []string{w32(b[:32]), w32(b[32:])}
}

func g2Words(p *bn254.G2Affine) []string {
	// Fe order: x0 (real), x1 (imag), y0, y1. gnark: A0 = real, A1 = imag.
	f := func(e big.Int) string { var b [32]byte; e.FillBytes(b[:]); return w32(b[:]) }
	var x0, x1, y0, y1 big.Int
	p.X.A0.BigInt(&x0)
	p.X.A1.BigInt(&x1)
	p.Y.A0.BigInt(&y0)
	p.Y.A1.BigInt(&y1)
	return []string{f(x0), f(x1), f(y0), f(y1)}
}

func points(ps []bn254.G1Affine) [][]string {
	out := [][]string{}
	for i := range ps {
		out = append(out, g1Words(&ps[i]))
	}
	return out
}

type challenges struct {
	Gamma        string   `json:"gamma"`
	Beta         string   `json:"beta"`
	Alpha        string   `json:"alpha"`
	Zeta         string   `json:"zeta"`
	GammaKzg     string   `json:"gamma_kzg"`
	Random       string   `json:"random"`
	GammaRaw     string   `json:"gamma_raw"`
	BetaRaw      string   `json:"beta_raw"`
	AlphaRaw     string   `json:"alpha_raw"`
	ZetaRaw      string   `json:"zeta_raw"`
	PI           string   `json:"pi"`
	OpeningLin   string   `json:"opening_lin"`
	HashedCommit string   `json:"hashed_commit"`
	LinDigest    []string `json:"lin_digest"`
	FoldedDigest []string `json:"folded_digest"`
	FoldedValue  string   `json:"folded_value"`
}

// replay recomputes every verifier-side value exactly as gnark v0.12.0 backend/plonk/bn254
// verify.go does (same library calls: fiat-shamir transcript with sha256, hash_to_field,
// kzg.FoldProof), independently of the Fe code. `random` (the batching scalar of the final
// pairing) is drawn by gnark's Go verifier with SetRandom; the Solidity verifier derives it as
// sha256 of (folded digest, W_zeta, [Z], W_zeta_omega, zeta, gamma_kzg); we replay the latter.
func replay(vk *plonkbn254.VerifyingKey, proof *plonkbn254.Proof, pub fr.Vector) challenges {
	fs := fiatshamir.NewTranscript(sha256.New(), "gamma", "beta", "alpha", "zeta")
	bind := func(name string, b []byte) {
		if err := fs.Bind(name, b); err != nil {
			panic(err)
		}
	}
	for _, s := range vk.S {
		bind("gamma", s.Marshal())
	}
	for _, p := range []bn254.G1Affine{vk.Ql, vk.Qr, vk.Qm, vk.Qo, vk.Qk} {
		bind("gamma", p.Marshal())
	}
	for i := range vk.Qcp {
		bind("gamma", vk.Qcp[i].Marshal())
	}
	for i := range pub {
		bind("gamma", pub[i].Marshal())
	}
	var raw [4][]byte
	derive := func(k int, name string, ps ...bn254.G1Affine) fr.Element {
		for i := range ps {
			b := ps[i].RawBytes()
			bind(name, b[:])
		}
		c, err := fs.ComputeChallenge(name)
		if err != nil {
			panic(err)
		}
		raw[k] = c
		var r fr.Element
		r.SetBytes(c)
		return r
	}
	gamma := derive(0, "gamma", proof.LRO[0], proof.LRO[1], proof.LRO[2])
	beta := derive(1, "beta")
	deps := append(append([]bn254.G1Affine{}, proof.Bsb22Commitments...), proof.Z)
	alpha := derive(2, "alpha", deps...)
	zeta := derive(3, "zeta", proof.H[0], proof.H[1], proof.H[2])

	one := fr.One()
	var zetaPowerM, zhZeta, lagrangeZero fr.Element
	var bExpo big.Int
	bExpo.SetUint64(vk.Size)
	zetaPowerM.Exp(zeta, &bExpo)
	zhZeta.Sub(&zetaPowerM, &one)
	lagrangeZero.Sub(&zeta, &one).Inverse(&lagrangeZero).Mul(&lagrangeZero, &zhZeta).Mul(&lagrangeZero, &vk.SizeInv)

	var pi, accw, xiLi fr.Element
	dens := make([]fr.Element, len(pub))
	accw.SetOne()
	for i := range pub {
		dens[i].Sub(&zeta, &accw)
		accw.Mul(&accw, &vk.Generator)
	}
	invDens := fr.BatchInvert(dens)
	accw.SetOne()
	for i := range pub {
		xiLi.Mul(&zhZeta, &invDens[i]).Mul(&xiLi, &vk.SizeInv).Mul(&xiLi, &accw).Mul(&xiLi, &pub[i])
		accw.Mul(&accw, &vk.Generator)
		pi.Add(&pi, &xiLi)
	}
	hashedStr := ""
	hf := hash_to_field.New([]byte("BSB22-Plonk"))
	for i, cci := range vk.CommitmentConstraintIndexes {
		hf.Write(proof.Bsb22Commitments[i].Marshal())
		hb := hf.Sum(nil)
		hf.Reset()
		var hashedCmt, wPowI, den, lagrange fr.Element
		hashedCmt.SetBytes(hb[:fr.Bytes])
		hashedStr = frHex(&hashedCmt)
		wPowI.Exp(vk.Generator, big.NewInt(int64(vk.NbPublicVariables)+int64(cci)))
		den.Sub(&zeta, &wPowI)
		lagrange.SetOne().Sub(&zetaPowerM, &lagrange).Mul(&lagrange, &wPowI).Div(&lagrange, &den).Mul(&lagrange, &vk.SizeInv)
		xiLi.Mul(&lagrange, &hashedCmt)
		pi.Add(&pi, &xiLi)
	}

	var _s1, _s2, tmp fr.Element
	l := proof.BatchedProof.ClaimedValues[1]
	r := proof.BatchedProof.ClaimedValues[2]
	o := proof.BatchedProof.ClaimedValues[3]
	s1 := proof.BatchedProof.ClaimedValues[4]
	s2 := proof.BatchedProof.ClaimedValues[5]
	zu := proof.ZShiftedOpening.ClaimedValue
	var a2l0 fr.Element
	a2l0.Mul(&lagrangeZero, &alpha).Mul(&a2l0, &alpha)
	var constLin fr.Element
	constLin.Mul(&beta, &s1).Add(&constLin, &gamma).Add(&constLin, &l)
	tmp.Mul(&s2, &beta).Add(&tmp, &gamma).Add(&tmp, &r)
	constLin.Mul(&constLin, &tmp)
	tmp.Add(&o, &gamma)
	constLin.Mul(&tmp, &constLin).Mul(&constLin, &alpha).Mul(&constLin, &zu)
	constLin.Sub(&constLin, &a2l0).Add(&constLin, &pi)
	constLin.Neg(&constLin)

	_s1.Mul(&beta, &s1).Add(&_s1, &l).Add(&_s1, &gamma)
	tmp.Mul(&beta, &s2).Add(&tmp, &r).Add(&tmp, &gamma)
	_s1.Mul(&_s1, &tmp).Mul(&_s1, &beta).Mul(&_s1, &alpha).Mul(&_s1, &zu)
	_s2.Mul(&beta, &zeta).Add(&_s2, &gamma).Add(&_s2, &l)
	tmp.Mul(&beta, &vk.CosetShift).Mul(&tmp, &zeta).Add(&tmp, &gamma).Add(&tmp, &r)
	_s2.Mul(&_s2, &tmp)
	tmp.Mul(&beta, &vk.CosetShift).Mul(&tmp, &vk.CosetShift).Mul(&tmp, &zeta).Add(&tmp, &o).Add(&tmp, &gamma)
	_s2.Mul(&_s2, &tmp).Mul(&_s2, &alpha).Neg(&_s2)
	var coeffZ, rl fr.Element
	coeffZ.Add(&a2l0, &_s2)
	rl.Mul(&l, &r)
	nPlusTwo := big.NewInt(int64(vk.Size) + 2)
	var zn2zh, zn2sqzh, zh fr.Element
	zn2zh.Exp(zeta, nPlusTwo)
	zn2sqzh.Mul(&zn2zh, &zn2zh)
	zn2zh.Mul(&zn2zh, &zhZeta).Neg(&zn2zh)
	zn2sqzh.Mul(&zn2sqzh, &zhZeta).Neg(&zn2sqzh)
	zh.Neg(&zhZeta)
	var lin bn254.G1Affine
	points := append(append([]bn254.G1Affine{}, proof.Bsb22Commitments...),
		vk.Ql, vk.Qr, vk.Qm, vk.Qo, vk.Qk, vk.S[2], proof.Z, proof.H[0], proof.H[1], proof.H[2])
	qC := make([]fr.Element, len(proof.Bsb22Commitments))
	copy(qC, proof.BatchedProof.ClaimedValues[6:])
	scalars := append(qC, l, r, rl, o, one, _s1, coeffZ, zh, zn2zh, zn2sqzh)
	_, err := lin.MultiExp(points, scalars, ecc.MultiExpConfig{})
	must(err)
	digests := make([]bn254.G1Affine, len(vk.Qcp)+6)
	copy(digests[6:], vk.Qcp)
	digests[0] = lin
	digests[1], digests[2], digests[3] = proof.LRO[0], proof.LRO[1], proof.LRO[2]
	digests[4], digests[5] = vk.S[0], vk.S[1]

	// gamma_kzg exactly as kzg.deriveGamma (sha256, name "gamma", data: zeta, digests, claimed
	// values, Z(zeta omega)).
	gfs := fiatshamir.NewTranscript(sha256.New(), "gamma")
	gb := func(b []byte) { must(gfs.Bind("gamma", b)) }
	gb(zeta.Marshal())
	for i := range digests {
		gb(digests[i].Marshal())
	}
	for i := range proof.BatchedProof.ClaimedValues {
		cv := proof.BatchedProof.ClaimedValues[i]
		if i == 0 {
			cv = constLin
		}
		gb(cv.Marshal())
	}
	gb(zu.Marshal())
	gc, err := gfs.ComputeChallenge("gamma")
	must(err)
	var gammaKzg fr.Element
	gammaKzg.SetBytes(gc)

	// check against gnark-crypto's own folding
	bp := proof.BatchedProof
	bp.ClaimedValues = append([]fr.Element{}, bp.ClaimedValues...)
	bp.ClaimedValues[0] = constLin
	foldedProof, foldedDigest, err := kzg.FoldProof(digests, &bp, zeta, sha256.New(), zu.Marshal())
	must(err)

	// random: sha256(folded digest, W_zeta, [Z], W_zeta_omega, zeta, gamma_kzg) as in the
	// Solidity verifier (0x140 bytes).
	var buf []byte
	for _, p := range []bn254.G1Affine{foldedDigest, proof.BatchedProof.H, proof.Z, proof.ZShiftedOpening.H} {
		b := p.RawBytes()
		buf = append(buf, b[:]...)
	}
	zb, gkb := zeta.Bytes(), gammaKzg.Bytes()
	buf = append(buf, zb[:]...)
	buf = append(buf, gkb[:]...)
	rh := sha256.Sum256(buf)
	var random fr.Element
	random.SetBytes(rh[:])

	return challenges{
		Gamma: frHex(&gamma), Beta: frHex(&beta), Alpha: frHex(&alpha), Zeta: frHex(&zeta),
		GammaKzg: frHex(&gammaKzg), Random: frHex(&random),
		GammaRaw: w32(raw[0]), BetaRaw: w32(raw[1]), AlphaRaw: w32(raw[2]), ZetaRaw: w32(raw[3]),
		PI: frHex(&pi), OpeningLin: frHex(&constLin), HashedCommit: hashedStr,
		LinDigest: g1Words(&lin), FoldedDigest: g1Words(&foldedDigest), FoldedValue: frHex(&foldedProof.ClaimedValue),
	}
}

func run(name string, circuit frontend.Circuit, assign frontend.Circuit, outDir, ptauPath string) {
	ccs, err := frontend.Compile(ecc.BN254.ScalarField(), scs.NewBuilder, circuit)
	must(err)
	n := ecc.NextPowerOfTwo(uint64(ccs.GetNbConstraints() + ccs.GetNbPublicVariables()))
	srs, srsLag := srsFromPtau(ptauPath, n)
	pk, vkI, err := plonk.Setup(ccs, srs, srsLag)
	must(err)
	w, err := frontend.NewWitness(assign, ecc.BN254.ScalarField())
	must(err)
	pw, err := w.Public()
	must(err)
	proofI, err := plonk.Prove(ccs, pk, w)
	must(err)
	must(plonk.Verify(proofI, vkI, pw))
	fmt.Println(name, ": gnark Go verifier accepts")

	vk := vkI.(*plonkbn254.VerifyingKey)
	proof := proofI.(*plonkbn254.Proof)
	pub := pw.Vector().(fr.Vector)

	must(os.MkdirAll(outDir, 0o755))
	f, err := os.Create(filepath.Join(outDir, "Verifier.sol"))
	must(err)
	must(vk.ExportSolidity(f))
	must(f.Close())

	ms := proof.MarshalSolidity()
	words := []string{}
	for i := 0; i < len(ms); i += 32 {
		words = append(words, w32(ms[i:i+32]))
	}
	pubs := []string{}
	for i := range pub {
		pubs = append(pubs, frHex(&pub[i]))
	}
	ch := replay(vk, proof, pub)
	var gen, sizeInv, cs fr.Element = vk.Generator, vk.SizeInv, vk.CosetShift
	out := map[string]any{
		"name":                 name,
		"gnark_version":        "v0.12.0",
		"srs":                  "PSE perpetual powers of tau ppot_0080_05 (real ceremony, sha256 " + PtauSHA256 + ")",
		"size":                 vk.Size,
		"size_inv":             frHex(&sizeInv),
		"generator":            frHex(&gen),
		"coset_shift":          frHex(&cs),
		"nb_public":            vk.NbPublicVariables,
		"s":                    points(vk.S[:]),
		"ql":                   g1Words(&vk.Ql),
		"qr":                   g1Words(&vk.Qr),
		"qm":                   g1Words(&vk.Qm),
		"qo":                   g1Words(&vk.Qo),
		"qk":                   g1Words(&vk.Qk),
		"qcp":                  points(vk.Qcp),
		"commit_constraint_ix": vk.CommitmentConstraintIndexes,
		"g1":                   g1Words(&vk.Kzg.G1),
		"g2":                   g2Words(&vk.Kzg.G2[0]),
		"g2_x":                 g2Words(&vk.Kzg.G2[1]),
		"public":               pubs,
		"proof_words":          words,
		"challenges":           ch,
	}
	j, err := json.MarshalIndent(out, "", "  ")
	must(err)
	must(os.WriteFile(filepath.Join(outDir, "fixture.json"), j, 0o644))
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}

func main() {
	out, ptau := os.Args[1], os.Args[2]
	run("mul", &Mul{}, &Mul{X: 3, Y: 5, Z: 22}, filepath.Join(out, "mul"), ptau)
	run("mulcommit", &MulCommit{}, &MulCommit{X: 3, Y: 5, Z: 22}, filepath.Join(out, "mulcommit"), ptau)
}
