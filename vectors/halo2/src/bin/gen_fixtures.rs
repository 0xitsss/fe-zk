//! Regenerates vectors/halo2/standard_plonk_lookup/*. vk is deterministic; proof bytes are not (see scripts/regen-halo2.sh).
use fe_zk_halo2_vectors::circuit::Demo;
use fe_zk_halo2_vectors::export::*;
use halo2_base::halo2_proofs::{
    halo2curves::bn256::{Bn256, Fq, Fr},
    poly::commitment::ParamsProver,
};
use serde_json::json;
use snark_verifier::{
    loader::evm::{self, deploy_and_call, encode_calldata, EvmLoader},
    pcs::kzg::{Bdfg21, Gwc19, KzgAs},
    system::halo2::{compile, transcript::evm::EvmTranscript, Config},
    verifier::{self, SnarkVerifier},
};
use std::{path::PathBuf, rc::Rc};

type PlonkGwc = verifier::plonk::PlonkVerifier<KzgAs<Bn256, Gwc19>>;
type PlonkShplonk = verifier::plonk::PlonkVerifier<KzgAs<Bn256, Bdfg21>>;

macro_rules! sol {
    ($pv:ty, $params:expr, $vk:expr) => {{
        let num_instance = Demo::num_instance();
        let protocol = compile($params, $vk, Config::kzg().with_num_instance(num_instance.clone()));
        let svk = ($params.get_g()[0], $params.g2(), $params.s_g2()).into();
        let loader = EvmLoader::new::<Fq, Fr>();
        let protocol = protocol.loaded(&loader);
        let mut transcript = EvmTranscript::<_, Rc<EvmLoader>, _, _>::new(&loader);
        let instances = transcript.load_instances(num_instance);
        let proof = <$pv>::read_proof(&svk, &protocol, &instances, &mut transcript).unwrap();
        <$pv>::verify(&svk, &protocol, &instances, &proof).unwrap();
        loader.solidity_code()
    }};
}

fn main() {
    let out: PathBuf =
        std::env::args().nth(1).map(Into::into).unwrap_or("standard_plonk_lookup".into());
    std::fs::create_dir_all(&out).unwrap();
    let params = setup_params();
    let pk = keygen(&params);
    let vk = pk.get_vk();
    let circuit = Demo::new(3, 5);
    mock_check(&circuit);
    let inst = circuit.instances();

    std::fs::write(
        out.join("vk.json"),
        serde_json::to_string_pretty(&vk_json(&params, &pk)).unwrap() + "\n",
    )
    .unwrap();

    let mut gas = serde_json::Map::new();
    for (name, shplonk) in [("shplonk", true), ("gwc", false)] {
        let proof = if shplonk {
            prove_shplonk(&params, &pk, circuit.clone(), &inst, 1)
        } else {
            prove_gwc(&params, &pk, circuit.clone(), &inst, 2)
        };
        let ok = if shplonk {
            verify_shplonk(&params, vk, &inst, &proof)
        } else {
            verify_gwc(&params, vk, &inst, &proof)
        };
        assert!(ok, "halo2 native verifier rejected {name}");
        // negative control: wrong instance must fail natively
        let bad = vec![vec![inst[0][0] + Fr::from(1u64)]];
        let ok_bad = if shplonk {
            verify_shplonk(&params, vk, &bad, &proof)
        } else {
            verify_gwc(&params, vk, &bad, &proof)
        };
        assert!(!ok_bad);
        let j = json!({
            "_doc": "halo2-KZG proof (EVM keccak transcript). proof = raw bytes hex; instances[column][row] as 0x Fr (big-endian).",
            "multiopen": name,
            "proof_len": proof.len(),
            "proof": format!("0x{}", hex::encode(&proof)),
            "instances": inst.iter().map(|c| c.iter().map(hex_fr).collect::<Vec<_>>()).collect::<Vec<_>>(),
        });
        std::fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_string_pretty(&j).unwrap() + "\n",
        )
        .unwrap();

        // snark-verifier Solidity baseline + gas
        let code = if shplonk {
            sol!(PlonkShplonk, &params, vk)
        } else {
            sol!(PlonkGwc, &params, vk)
        };
        std::fs::write(out.join(format!("snark_verifier_{name}.sol")), &code).unwrap();
        let deploy = evm::compile_solidity(&code.replace("pragma solidity 0.8.19;", "pragma solidity ^0.8.19;"));
        let calldata = encode_calldata(&inst, &proof);
        match deploy_and_call(deploy, calldata) {
            Ok(g) => {
                println!("snark-verifier {name} EVM verify gas: {g}");
                gas.insert(name.into(), json!(g));
            }
            Err(e) => println!("snark-verifier {name} EVM failed: {e:?}"),
        }
        println!("{name}: proof {} bytes ok", proof.len());
    }
    std::fs::write(
        out.join("snark_verifier_gas.json"),
        serde_json::to_string_pretty(&json!({
            "_doc": "gas reported by snark_verifier::loader::evm::deploy_and_call (revm) for the snark-verifier 0.2.7 generated Solidity verifier of this circuit (call only, incl. tx base cost as reported by revm)",
            "gas": gas }))
        .unwrap()
            + "\n",
    )
    .unwrap();
}
