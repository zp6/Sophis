use risc0_zkvm::{InnerReceipt, Receipt};

/// Verify a Risc0 STARK proof offline (no re-execution).
///
/// - `seal`:     raw bytes of a STARK-based Risc0 seal produced by the prover
///               (`InnerReceipt::Composite` or `InnerReceipt::Succinct`).
/// - `journal`:  public output bytes (borsh-encoded, as committed by `env::commit_slice`).
/// - `image_id`: 32-byte image ID of the expected guest ELF.
///
/// Sophis is PQC-only at the consensus layer, so any Risc0 receipt variant
/// that depends on elliptic-curve pairings (`InnerReceipt::Groth16`, which
/// wraps the STARK proof in a BN254 Groth16 verifier) is rejected here even
/// if it would otherwise verify. `InnerReceipt::Fake` is also rejected: dev-
/// only shortcuts must not survive into chain validation.
///
/// Returns `true` if the proof is a valid STARK seal for the given
/// `image_id` and `journal`. Returns `false` on any malformed input,
/// disallowed receipt variant, or verification failure.
pub fn verify_risc0_proof_bytes(seal: &[u8], journal: &[u8], image_id: &[u8]) -> bool {
    if image_id.len() != 32 {
        return false;
    }
    let Ok(id_bytes) = <[u8; 32]>::try_from(image_id) else {
        return false;
    };
    // Convert [u8; 32] → [u32; 8] (Risc0 image ID format: 8× big-endian u32)
    let mut image_id_words = [0u32; 8];
    for (i, chunk) in id_bytes.chunks_exact(4).enumerate() {
        image_id_words[i] = u32::from_be_bytes(chunk.try_into().unwrap());
    }

    // Attempt to deserialize the receipt from the seal bytes.
    // Risc0 seals are bincode-serialized InnerReceipt.
    let inner: InnerReceipt = match bincode::deserialize(seal) {
        Ok(r) => r,
        Err(_) => return false,
    };

    // PQC-only gate: only the STARK-based receipt variants are eligible for
    // Sophis chain validation. Groth16 wraps the STARK proof in a BN254
    // pairing-based verifier (not post-quantum); Fake exists for dev only;
    // any future non-STARK variant is rejected by the catch-all arm.
    match &inner {
        InnerReceipt::Composite(_) | InnerReceipt::Succinct(_) => {
            let receipt = Receipt::new(inner, journal.to_vec());
            receipt.verify(image_id_words).is_ok()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_image_id_length_rejected() {
        assert!(!verify_risc0_proof_bytes(&[], &[], &[0u8; 31]));
        assert!(!verify_risc0_proof_bytes(&[], &[], &[0u8; 33]));
    }

    #[test]
    fn garbage_seal_rejected() {
        assert!(!verify_risc0_proof_bytes(b"garbage", b"journal", &[0u8; 32]));
    }
}
