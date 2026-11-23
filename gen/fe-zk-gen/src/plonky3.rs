//! `fe-zk-gen plonky3`: Plonky3 `p3-uni-stark` 0.8.0 FRI/STARK proofs to Fe.
//!
//! Rust port of `vectors/plonky3-stark/emit_fe.py` (kept as a byte-identical cross-check, see
//! `tests/plonky3.rs`). Input: a directory with `proof.json`, `config.json` and `vectors.json`
//! written by the fixture crate `vectors/plonky3-stark`. Output: a per-configuration module
//! (`FriParams`, proof lengths, `verify`, optional compile-time soundness assert) and a test
//! fixture (flat proof words, public values, Fiat-Shamir challenges printed by Plonky3).
//!
//! Only data is converted. Every shape assumption of the Fe verifier (`src/verifier/fri.fe`) is
//! checked and reported as an error instead of emitting a verifier for another shape: no
//! randomization or preprocessed trace, one quotient chunk, Merkle cap height 0, zero-bit batch
//! and out-of-domain grinding with zero witnesses, the domain-separator seeds Plonky3 printed
//! must be the ones the Fe verifier rebuilds from the configuration and the pattern hashes.

use serde_json::Value;
use std::fmt::Write;

/// KoalaBear modulus `2^31 - 2^24 + 1`.
const P: u64 = 0x7F00_0001;

/// Options of one emission (the arguments of `emit_fe.py`).
#[derive(Clone, Debug)]
pub struct Plonky3EmitOptions {
    /// Directory label written into the headers (as given on the command line).
    pub dir: String,
    /// Configuration name (`demo`, `secure`, ...), only used in comments.
    pub name: String,
    /// `fib` or `mul`: the AIR of `src/verifier/fri/air_*.fe`.
    pub air: String,
    /// Emit the extension-field and challenger vectors into the fixture.
    pub vectors: bool,
    /// Require (at generation time and with a Fe `static_assert`) at least this many conjectured bits.
    pub min_bits: Option<u64>,
}

/// The two emitted files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plonky3Emitted {
    pub fixture_fe: String,
    pub config_fe: String,
}

type R<T> = Result<T, String>;

fn ensure(c: bool, msg: &str) -> R<()> {
    if c {
        Ok(())
    } else {
        Err(msg.to_string())
    }
}

fn get<'a>(v: &'a Value, k: &str) -> R<&'a Value> {
    v.get(k).ok_or_else(|| format!("missing field `{k}`"))
}

fn u(v: &Value) -> R<u64> {
    v.as_u64().ok_or_else(|| format!("expected an unsigned integer, got {v}"))
}

fn arr(v: &Value) -> R<&Vec<Value>> {
    v.as_array().ok_or_else(|| format!("expected an array, got {v}"))
}

fn s(v: &Value) -> R<&str> {
    v.as_str().ok_or_else(|| format!("expected a string, got {v}"))
}

fn hx(v: &Value) -> R<String> {
    let t = s(v)?;
    ensure(
        t.starts_with("0x") && t.len() == 66 && t[2..].bytes().all(|b| b.is_ascii_hexdigit()),
        "expected a 32-byte 0x-hex string",
    )?;
    Ok(t.to_string())
}

fn modpow(mut b: u64, mut e: u64) -> u64 {
    let mut r = 1u64;
    b %= P;
    while e > 0 {
        if e & 1 == 1 {
            r = r * b % P;
        }
        b = b * b % P;
        e >>= 1;
    }
    r
}

/// serde_json of a Plonky3 KoalaBear is its Montgomery form (`value * 2^32 mod p`).
fn mont(v: &Value) -> R<u64> {
    let x = u(v)?;
    ensure(x < P, "field element out of range")?;
    let r_inv = modpow((1u64 << 32) % P, P - 2);
    Ok(x * r_inv % P)
}

fn join(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

fn ints(xs: &[Value]) -> R<String> {
    Ok(join(&xs.iter().map(|x| u(x).map(|n| n.to_string())).collect::<R<Vec<_>>>()?))
}

fn ext_list(xs: &[Value]) -> R<String> {
    Ok(join(&xs.iter().map(|x| ints(arr(x)?)).collect::<R<Vec<_>>>()?))
}

/// A Merkle commitment (`MerkleCap`, cap height 0): its single root.
fn root(com: &Value) -> R<String> {
    let cap = arr(get(com, "cap")?)?;
    ensure(cap.len() == 1, "only Merkle cap height 0 is supported")?;
    hx(&cap[0])
}

/// Plonky3 0.8 `DomainSeparator::seed` for a KoalaBear alphabet: the field elements absorbed.
fn seed_felts(name: &[u8], pattern_hash: &[u8], label: &[u64]) -> Vec<u64> {
    let mut seed = vec![1u8];
    seed.extend_from_slice(name);
    seed.extend(std::iter::repeat(0).take(62 - name.len()));
    seed.push(name.len() as u8);
    let mut lab = vec![];
    for v in label {
        lab.extend_from_slice(&8u32.to_be_bytes());
        lab.extend_from_slice(&v.to_be_bytes());
    }
    seed.extend_from_slice(pattern_hash);
    seed.extend_from_slice(&(lab.len() as u32).to_be_bytes());
    seed.extend_from_slice(&lab);
    seed.push(0x80);
    let mut out = vec![seed.len() as u64];
    for c in seed.chunks(3) {
        out.push(c.iter().enumerate().map(|(i, &b)| (b as u64) << (8 * i)).sum());
    }
    out
}

/// `(Fe module in verifier/fri, Fe type, constant prefix of the fixture)` of an AIR name.
fn air_info(air: &str) -> R<(&'static str, &'static str, &'static str)> {
    match air {
        "fib" => Ok(("air_fib", "FibonacciAir", "FIB")),
        "mul" => Ok(("air_mul", "MulAir", "MUL")),
        _ => Err(format!("unknown AIR `{air}` (fib, mul)")),
    }
}

/// Emit the configuration module and the fixture for one proof directory.
pub fn emit_plonky3(proof: &Value, cfg: &Value, vec: &Value, o: &Plonky3EmitOptions) -> R<Plonky3Emitted> {
    let (air_mod, air_ty, pre) = air_info(&o.air)?;
    let width = match cfg.get("trace_width") {
        Some(w) => u(w)?,
        None => 2,
    };
    let public = arr(get(cfg, "public_values")?)?;
    let npub = public.len();
    let log_n = u(get(cfg, "log_trace_rows")?)?;
    let log_b = u(get(cfg, "log_blowup")?)?;
    let log_f = u(get(cfg, "log_final_poly_len")?)?;
    let nq = u(get(cfg, "num_queries")?)?;
    ensure(log_f < log_n && log_n + log_b <= 24 && log_f <= 8, "unsupported trace / final polynomial size")?;
    ensure((1..=128).contains(&nq), "unsupported number of queries")?;
    ensure((1..=64).contains(&width), "unsupported trace width")?;
    let f = log_n - log_f;
    let lh = log_n + log_b;

    let ov = get(proof, "opened_values")?;
    let op = get(proof, "opening_proof")?;
    ensure(get(get(proof, "commitments")?, "random")?.is_null(), "randomized (zk) proofs are not supported")?;
    ensure(
        get(ov, "preprocessed_local")?.is_null() && get(ov, "preprocessed_next")?.is_null(),
        "preprocessed traces are not supported",
    )?;
    let qc = arr(get(ov, "quotient_chunks")?)?;
    ensure(get(ov, "random")?.is_null() && qc.len() == 1, "exactly one quotient chunk is supported")?;
    ensure(u(get(proof, "degree_bits")?)? == log_n, "degree_bits does not match the configuration")?;
    ensure(
        u(get(proof, "ood_pow_witness")?)? == 0 && u(get(op, "batch_pow_witness")?)? == 0,
        "nonzero out-of-domain or batch PoW witness (zero-bit sites)",
    )?;
    let commits = arr(get(op, "commit_phase_commits")?)?;
    let cpo = arr(get(op, "commit_phase_openings")?)?;
    let final_poly = arr(get(op, "final_poly")?)?;
    let cpw = arr(get(op, "commit_pow_witnesses")?)?;
    ensure(commits.len() as u64 == f && cpo.len() as u64 == f, "wrong number of fold rounds")?;
    ensure(final_poly.len() as u64 == 1 << log_f, "wrong final polynomial length")?;
    ensure(cpw.len() as u64 == f, "wrong number of commit-phase PoW witnesses")?;
    let ins = arr(get(op, "input_openings")?)?;
    ensure(ins.len() == 2, "expected two input batches (trace, quotient)")?;

    // ---- flat proof words, in the layout documented in src/verifier/fri.fe
    let mut words: Vec<String> = vec![];
    let felts = |words: &mut Vec<String>, xs: &[Value]| -> R<()> {
        for x in xs {
            words.push(mont(x)?.to_string());
        }
        Ok(())
    };
    let exts = |words: &mut Vec<String>, xs: &[Value]| -> R<()> {
        for e in xs {
            let e = arr(e)?;
            ensure(e.len() == 4, "extension element is not 4 coefficients")?;
            felts(words, e)?;
        }
        Ok(())
    };
    let coms = get(proof, "commitments")?;
    words.push(root(get(coms, "trace")?)?);
    words.push(root(get(coms, "quotient_chunks")?)?);
    exts(&mut words, arr(get(ov, "trace_local")?)?)?;
    exts(&mut words, arr(get(ov, "trace_next")?)?)?;
    exts(&mut words, arr(&qc[0])?)?;
    for c in commits {
        words.push(root(c)?);
    }
    felts(&mut words, cpw)?;
    exts(&mut words, final_poly)?;
    felts(&mut words, std::slice::from_ref(get(op, "query_pow_witness")?))?;
    for b in ins {
        ensure(arr(get(b, "opened_values")?)?.len() as u64 == nq, "wrong number of input openings")?;
    }
    for r in cpo {
        ensure(arr(get(r, "sibling_values")?)?.len() as u64 == nq, "wrong number of sibling values")?;
    }
    let one = |v: &Value| -> R<Value> {
        let a = arr(v)?;
        ensure(a.len() == 1, "expected exactly one opened matrix")?;
        Ok(a[0].clone())
    };
    for q in 0..nq as usize {
        let tr_row = one(&arr(get(&ins[0], "opened_values")?)?[q])?;
        let qu_row = one(&arr(get(&ins[1], "opened_values")?)?[q])?;
        ensure(
            arr(&tr_row)?.len() as u64 == width && arr(&qu_row)?.len() == 4,
            "opened row width does not match the configuration",
        )?;
        felts(&mut words, arr(&tr_row)?)?;
        felts(&mut words, arr(&qu_row)?)?;
        for r in cpo {
            let sib = one(&arr(get(r, "sibling_values")?)?[q])?;
            exts(&mut words, std::slice::from_ref(&sib))?;
        }
    }
    // pruned Merkle multi-openings: trace, quotient, then one per fold round
    let mut mps: Vec<&Value> = vec![get(&ins[0], "opening_proof")?, get(&ins[1], "opening_proof")?];
    for r in cpo {
        mps.push(get(r, "opening_proof")?);
    }
    let mut counts = vec![];
    for (k, mp) in mps.iter().enumerate() {
        let hs = arr(get(mp, "sibling_hashes")?)?;
        let levels = if k < 2 { lh } else { lh - 1 - (k as u64 - 2) };
        ensure(hs.len() as u64 <= nq * levels, "too many Merkle siblings")?;
        counts.push(hs.len() as u64);
    }
    words.extend(counts.iter().map(|n| n.to_string()));
    for mp in &mps {
        for h in arr(get(mp, "sibling_hashes")?)? {
            words.push(hx(h)?);
        }
    }

    // proof length from the layout formula (must equal src/verifier/fri.fe `proof_words`)
    let qw = width + 4 + 4 * f;
    let sum_counts: u64 = counts.iter().sum();
    let total = 18 + 8 * width + 2 * f + 4 * (1 << log_f) + 1 + nq * qw + (2 + f) + sum_counts;
    ensure(words.len() as u64 == total, "proof length does not match the layout")?;
    let fixed = total - sum_counts;
    let maxw = fixed + nq * (2 * lh + (0..f).map(|j| lh - 1 - j).sum::<u64>());

    // ---- transcript patterns (domain separators), checked against Plonky3's own seed elements
    let seeds = get(get(vec, "transcript")?, "seeds")?;
    let mut fri_label = vec![log_b, 1];
    fri_label.extend(std::iter::repeat(1).take(f as usize));
    let labels: [(&str, &[u8], Vec<u64>); 3] = [
        ("stark", b"p3-uni-stark", vec![log_n, log_n, width, 0, 0, 1, 1, 0]),
        ("pcs", b"p3-fri-pcs", vec![2, 1, 2, 1, 1]),
        ("fri", b"p3-fri", fri_label),
    ];
    let mut patterns = vec![];
    for (key, pname, label) in &labels {
        let sd = get(seeds, key)?;
        let ph = hx(get(sd, "pattern_hash")?)?;
        let bytes = hex::decode(&ph[2..]).map_err(|e| e.to_string())?;
        let want = arr(get(sd, "felts")?)?.iter().map(u).collect::<R<Vec<_>>>()?;
        ensure(
            seed_felts(pname, &bytes, label) == want,
            &format!("{key} domain separator does not match the Fe verifier's seed"),
        )?;
        patterns.push(ph);
    }

    let snd = get(cfg, "soundness")?;
    let conj = u(get(snd, "conjectured_bits")?)?;
    let qpow = u(get(cfg, "query_proof_of_work_bits")?)?;
    let cpow = u(get(cfg, "commit_proof_of_work_bits")?)?;
    ensure(conj == log_b * nq + qpow, "conjectured_bits does not match the configuration")?;
    if let Some(m) = o.min_bits {
        ensure(conj >= m, &format!("conjectured {conj} < required {m}"))?;
    }
    let d = o.dir.trim_end_matches('/');

    // ---- configuration module
    let mut c = String::new();
    let w = &mut c;
    let l = |w: &mut String, t: &str| {
        w.push_str(t);
        w.push('\n');
    };
    l(w, &format!("//! GENERATED by fe-zk-gen plonky3 (vectors/plonky3-stark/emit_fe.py) from {d}/config.json. Do not edit."));
    l(w, &format!("//! FRI/STARK configuration `{}`: trace 2^{log_n} rows, log_blowup {log_b}, final polynomial", o.name));
    l(w, &format!("//! 2^{log_f} coefficients, {nq} queries, commit PoW {cpow} bits, query PoW"));
    l(w, &format!("//! {qpow} bits. Conjectured soundness (Plonky3 `log_blowup * num_queries + query_pow_bits`):"));
    l(w, &format!("//! {conj} bits; proven estimates in `docs/DESIGN.md`."));
    l(w, "use core::ptr::MemArray");
    l(w, "use ingot::verifier::fri::FriParams");
    l(w, "use ingot::verifier::fri::verify as verify_with");
    l(w, "use ingot::verifier::fri::public_values");
    l(w, &format!("use ingot::verifier::fri::{air_mod}::{air_ty}"));
    l(w, "");
    l(w, "pub const PARAMS: FriParams = FriParams {");
    l(w, &format!("    width: {width},"));
    l(w, &format!("    log_n: {log_n},"));
    l(w, &format!("    log_blowup: {log_b},"));
    l(w, &format!("    log_final: {log_f},"));
    l(w, &format!("    num_queries: {nq},"));
    l(w, &format!("    commit_pow_bits: {cpow},"));
    l(w, &format!("    query_pow_bits: {qpow},"));
    l(w, &format!("    stark_pattern: {},", patterns[0]));
    l(w, &format!("    pcs_pattern: {},", patterns[1]));
    l(w, &format!("    fri_pattern: {},", patterns[2]));
    l(w, "}");
    l(w, "/// Proof length without the pruned Merkle siblings, and the largest valid length (every");
    l(w, "/// multi-opening unpruned). The length of a proof is fixed by the sibling counts it declares");
    l(w, "/// (`ingot::verifier::fri::proof_words`).");
    l(w, &format!("pub const FIXED_PROOF_WORDS: usize = {fixed}"));
    l(w, &format!("pub const MAX_PROOF_WORDS: usize = {maxw}"));
    l(w, "/// Plonky3 `FriParameters::conjectured_soundness_bits` of this configuration.");
    l(w, &format!("pub const CONJECTURED_BITS: u256 = {conj}"));
    if let Some(m) = o.min_bits {
        l(w, &format!("static_assert(CONJECTURED_BITS >= {m})"));
    }
    l(w, "");
    l(w, "/// `proof` is `proof_words(PARAMS, proof)` words, at most `MAX_PROOF_WORDS` (`MemArray`: see");
    l(w, "/// `docs/DESIGN.md`).");
    l(w, &format!("pub fn verify(_ proof: MemArray<u256>, _ public: [u256; {npub}]) -> bool {{"));
    l(w, &format!("    verify_with<{air_ty}>(PARAMS, proof, public_values(public))"));
    l(w, "}");

    // ---- fixture module
    let mut fx = String::new();
    let w = &mut fx;
    l(w, &format!("//! GENERATED by fe-zk-gen plonky3 (vectors/plonky3-stark/emit_fe.py) from {d}/*.json. Do not edit."));
    l(w, &format!("//! Plonky3 p3-uni-stark 0.8.0 proof of the {air_ty} (flat words), values printed by Plonky3."));
    l(w, "use core::ptr::MemArray");
    l(w, "");
    if o.vectors {
        let ev = get(vec, "ext")?;
        let keys = ["a", "b", "add", "sub", "mul", "sq", "inv", "pow_1000003"];
        let cases = arr(get(ev, "cases")?)?;
        l(w, &format!("pub const EXT_CASES: [[[u256; 4]; 8]; {}] = [", cases.len()));
        for cs in cases {
            let row = keys.iter().map(|k| get(cs, k).cloned()).collect::<R<Vec<_>>>()?;
            l(w, &format!("    {},", ext_list(&row)?));
        }
        l(w, "]");
        let tag = arr(get(ev, "two_adic_generators")?)?;
        l(w, &format!("pub const TWO_ADIC_GENERATORS: [u256; {}] = {}", tag.len(), ints(tag)?));
        l(w, &format!("pub const TWO_ADIC_GENERATOR_24: u256 = {}", u(get(ev, "two_adic_generator_24")?)?));
        l(w, &format!("pub const GENERATOR: u256 = {}", u(get(ev, "generator")?)?));
        l(w, "");
        let ch = get(vec, "challenger")?;
        let ops = arr(get(ch, "ops")?)?;
        let names = ops.iter().map(|x| get(x, "op").and_then(s)).collect::<R<Vec<_>>>()?;
        let mut expect = vec!["observe_felts", "sample_ext", "observe_hash", "sample_bits"];
        expect.extend(std::iter::repeat("sample_ext").take(6));
        expect.extend(["grind_check", "sample_ext"]);
        ensure(names == expect, "challenger script changed: update the Fe test")?;
        let outs = arr(get(ch, "outputs")?)?;
        let exts_out: Vec<Value> = outs.iter().filter(|x| x.is_array()).cloned().collect();
        let bits: Vec<&Value> = outs.iter().filter(|x| !x.is_array()).collect();
        ensure(bits.len() == 1, "expected one sample_bits output")?;
        l(w, &format!("pub const CHAL_FELTS: [u256; 4] = {}", ints(arr(get(&ops[0], "vals")?)?)?));
        l(w, &format!("pub const CHAL_HASH: u256 = {}", hx(get(&ops[2], "hash")?)?));
        l(w, &format!("pub const CHAL_BITS: u256 = {}", u(bits[0])?));
        l(w, &format!("pub const CHAL_GRIND_BITS: u256 = {}", u(get(&ops[10], "bits")?)?));
        l(w, &format!("pub const CHAL_WITNESS: u256 = {}", u(get(&ops[10], "witness")?)?));
        l(w, &format!("pub const CHAL_EXT: [[u256; 4]; {}] = {}", exts_out.len(), ext_list(&exts_out)?));
        l(w, "");
    }
    let tr = get(vec, "transcript")?;
    ensure(get(tr, "pow_ok")?.as_bool() == Some(true), "Plonky3's transcript replay failed the PoW check")?;
    l(w, &format!("pub const TR_ALPHA: [u256; 4] = {}", ints(arr(get(tr, "alpha")?)?)?));
    l(w, &format!("pub const TR_ZETA: [u256; 4] = {}", ints(arr(get(tr, "zeta")?)?)?));
    l(w, &format!("pub const TR_FRI_ALPHA: [u256; 4] = {}", ints(arr(get(tr, "fri_alpha")?)?)?));
    l(w, &format!("pub const TR_BETAS: [[u256; 4]; {f}] = {}", ext_list(arr(get(tr, "betas")?)?)?));
    l(w, &format!("pub const TR_INDICES: [u256; {nq}] = {}", ints(arr(get(tr, "indices")?)?)?));
    l(w, "");
    l(w, &format!("pub const {pre}_PUBLIC: [u256; {npub}] = {}", ints(public)?));
    l(w, &format!("pub const {pre}_PROOF_WORDS: usize = {total}"));
    l(w, "");
    l(w, "/// The proof as a runtime-length memory array (what `verify` takes).");
    l(w, &format!("pub fn {}_proof() -> MemArray<u256> {{", pre.to_lowercase()));
    l(w, &format!("    let words: [u256; {total}] = ["));
    for chunk in words.chunks(4) {
        let mut line = String::from("        ");
        write!(line, "{},", chunk.join(", ")).unwrap();
        l(w, &line);
    }
    l(w, "    ]");
    l(w, &format!("    var m = unsafe {{ MemArray<u256>::new_uninit({total}) }}"));
    l(w, "    var i: usize = 0");
    l(w, &format!("    while i < {total} {{"));
    l(w, "        m[i] = words[i]");
    l(w, "        i += 1");
    l(w, "    }");
    l(w, "    m");
    l(w, "}");
    Ok(Plonky3Emitted { fixture_fe: fx, config_fe: c })
}

/// Read `<dir>/{proof,config,vectors}.json` and emit.
pub fn emit_plonky3_dir(dir: &std::path::Path, o: &Plonky3EmitOptions) -> R<Plonky3Emitted> {
    let load = |n: &str| -> R<Value> {
        let p = dir.join(n);
        let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        serde_json::from_str(&t).map_err(|e| format!("{}: {e}", p.display()))
    };
    emit_plonky3(&load("proof.json")?, &load("config.json")?, &load("vectors.json")?, o)
}
