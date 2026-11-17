//! Prints Plonky3's default KoalaBear Poseidon2 width-16 parameters and KATs as JSON.
use p3_field::{PrimeCharacteristicRing, PrimeField32};
use p3_koala_bear::*;
use p3_poseidon2::GenericPoseidon2LinearLayers;
use p3_symmetric::{Permutation, PseudoCompressionFunction, TruncatedPermutation};

fn u(x: &KoalaBear) -> u32 {
    x.as_canonical_u32()
}

fn main() {
    let perm = default_koalabear_poseidon2_16();

    // Internal matrix is `1 + diag(d)`: apply the generic internal layer to unit vectors,
    // so the diagonal comes from Plonky3's code and not from our reading of it.
    let mut diag = vec![];
    for i in 0..16 {
        let mut s = [KoalaBear::ZERO; 16];
        s[i] = KoalaBear::ONE;
        GenericPoseidon2LinearLayersKoalaBear::internal_linear_layer(&mut s);
        for (j, v) in s.iter().enumerate() {
            if j != i {
                assert_eq!(u(v), 1, "internal matrix off-diagonal must be 1");
            }
        }
        diag.push(u(&(s[i] - KoalaBear::ONE)));
    }

    let mut kats = vec![];
    let inputs: Vec<[u32; 16]> = vec![
        core::array::from_fn(|i| i as u32),
        [0; 16],
        core::array::from_fn(|i| 0x7f000000 - i as u32),
        core::array::from_fn(|i| ((i as u32 + 1) * 0x01234567) % 0x7f000001),
    ];
    for inp in inputs {
        let mut st: [KoalaBear; 16] = core::array::from_fn(|i| KoalaBear::new(inp[i]));
        perm.permute_mut(&mut st);
        let out: Vec<u32> = st.iter().map(u).collect();
        kats.push((inp.to_vec(), out));
    }

    // Merkle tree of 8 leaf digests (8 elements each) with Plonky3's TruncatedPermutation
    // compression (state = left ++ right, output = first 8 lanes). Leaf i, lane j = 1000*i+j+1.
    let comp = TruncatedPermutation::<_, 2, 8, 16>::new(perm.clone());
    let mut level: Vec<[KoalaBear; 8]> = (0..8u32)
        .map(|i| core::array::from_fn(|j| KoalaBear::new(1000 * i + j as u32 + 1)))
        .collect();
    let leaves = level.clone();
    let mut levels = vec![level.clone()];
    while level.len() > 1 {
        level = level.chunks(2).map(|c| comp.compress([c[0], c[1]])).collect();
        levels.push(level.clone());
    }
    let root = level[0];
    // authentication path of leaf 5: siblings bottom-up
    let mut path = vec![];
    let mut idx = 5usize;
    for lv in &levels[..3] {
        path.push(lv[idx ^ 1]);
        idx >>= 1;
    }
    let d8 = |x: &[KoalaBear; 8]| -> Vec<u32> { x.iter().map(u).collect() };
    let merkle = format!(
        "{{\"leaf_index\": 5, \"leaf\": {:?}, \"path\": {:?}, \"root\": {:?}, \"all_leaves\": {:?}}}",
        d8(&leaves[5]),
        path.iter().map(d8).collect::<Vec<_>>(),
        d8(&root),
        leaves.iter().map(d8).collect::<Vec<_>>()
    );

    let arr = |v: Vec<u32>| format!("{:?}", v);
    let ext = |c: &[[KoalaBear; 16]; 4]| {
        format!(
            "[{}]",
            c.iter()
                .map(|r| arr(r.iter().map(u).collect()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    println!("{{");
    println!("  \"source\": \"p3-koala-bear 0.9.0-rc.2 default_koalabear_poseidon2_16\",");
    println!("  \"width\": 16, \"sbox_degree\": 3, \"rounds_f\": 8, \"rounds_p\": 20,");
    println!("  \"external_initial\": {},", ext(&KOALABEAR_POSEIDON2_RC_16_EXTERNAL_INITIAL));
    println!("  \"external_final\": {},", ext(&KOALABEAR_POSEIDON2_RC_16_EXTERNAL_FINAL));
    println!("  \"internal\": {},", arr(KOALABEAR_POSEIDON2_RC_16_INTERNAL.iter().map(u).collect()));
    println!("  \"internal_diag\": {},", arr(diag));
    println!("  \"merkle\": {},", merkle);
    println!("  \"kats\": [");
    let n = kats.len();
    for (i, (a, b)) in kats.iter().enumerate() {
        println!(
            "    {{\"input\": {:?}, \"output\": {:?}}}{}",
            a,
            b,
            if i + 1 < n { "," } else { "" }
        );
    }
    println!("  ]");
    println!("}}");
}
