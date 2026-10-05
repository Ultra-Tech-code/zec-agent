use sapling_crypto::prover::{SpendProver, OutputProver};
use sapling_crypto::{
    circuit, Diversifier, Rseed, ProofGenerationKey, PaymentAddress, MerklePath
};
use sapling_crypto::value::{NoteValue, ValueCommitTrapdoor};
use sapling_crypto::keys::EphemeralSecretKey;
use rand_core::RngCore;

pub struct DummySpendProver;
impl SpendProver for DummySpendProver {
    type Proof = [u8; 192];
    fn prepare_circuit(
        _proof_generation_key: ProofGenerationKey,
        _diversifier: Diversifier,
        _rseed: Rseed,
        _value: NoteValue,
        _alpha: jubjub::Fr,
        _rcv: ValueCommitTrapdoor,
        _anchor: bls12_381::Scalar,
        _merkle_path: MerklePath,
    ) -> Option<circuit::Spend> {
        panic!("Sapling proving is not supported in this Ironwood-only signer");
    }
    fn create_proof<R: RngCore>(&self, _circuit: circuit::Spend, _rng: &mut R) -> Self::Proof {
        panic!("Sapling proving is not supported in this Ironwood-only signer");
    }
    fn encode_proof(proof: Self::Proof) -> [u8; 192] {
        proof
    }
}

pub struct DummyOutputProver;
impl OutputProver for DummyOutputProver {
    type Proof = [u8; 192];
    fn prepare_circuit(
        _esk: &EphemeralSecretKey,
        _payment_address: PaymentAddress,
        _rcm: jubjub::Fr,
        _value: NoteValue,
        _rcv: ValueCommitTrapdoor,
    ) -> circuit::Output {
        panic!("Sapling proving is not supported in this Ironwood-only signer");
    }
    fn create_proof<R: RngCore>(&self, _circuit: circuit::Output, _rng: &mut R) -> Self::Proof {
        panic!("Sapling proving is not supported in this Ironwood-only signer");
    }
    fn encode_proof(proof: Self::Proof) -> [u8; 192] {
        proof
    }
}
