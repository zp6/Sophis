//! L3 — Block commitment level types.
//!
//! Consensus-side types for the per-block commitment-level abstraction
//! exposed by the `getBlockCommitment` RPC. Mirror the RPC types but
//! use `sophis_hashes::Hash` directly. See `docs/L3_COMMITMENT_DESIGN.md`
//! for the canonical specification.

use serde::{Deserialize, Serialize};
use sophis_hashes::Hash;

/// Per-network constant: a block becomes `Confirmed` once it is at
/// least this many blocks deep. Frozen ABI — promotion to per-network
/// `Params` would require a SIP. 100 blocks @ 10 BPS ≈ 10 seconds.
pub const CONFIRMED_DEPTH_BLOCKS: u64 = 100;

/// Discrete commitment level. `Pending` means the block is known to the
/// node but is OFF the GHOSTDAG selected chain (could re-join on a
/// reorg). `Accepted` / `Confirmed` / `Finalized` are progressively
/// stronger guarantees about chain inclusion + depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum CommitmentLevel {
    Pending = 0,
    Accepted = 1,
    Confirmed = 2,
    Finalized = 3,
}

impl CommitmentLevel {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Per-block commitment status. Returned by `ConsensusApi::get_block_commitment`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockCommitment {
    pub block_hash: Hash,
    pub block_blue_score: u64,
    pub current_blue_score: u64,
    pub confirmations: u64,
    pub is_chain_block: bool,
    pub commitment: CommitmentLevel,
}

impl BlockCommitment {
    /// Builds a `BlockCommitment` by classifying the inputs per design
    /// §3.3. Pure helper: takes already-resolved fields, returns the
    /// classified record.
    pub fn classify(
        block_hash: Hash,
        block_blue_score: u64,
        current_blue_score: u64,
        is_chain_block: bool,
        finality_depth: u64,
    ) -> Self {
        let confirmations = current_blue_score.saturating_sub(block_blue_score);
        let commitment = if !is_chain_block {
            CommitmentLevel::Pending
        } else if confirmations >= finality_depth {
            CommitmentLevel::Finalized
        } else if confirmations >= CONFIRMED_DEPTH_BLOCKS {
            CommitmentLevel::Confirmed
        } else {
            CommitmentLevel::Accepted
        };
        Self { block_hash, block_blue_score, current_blue_score, confirmations, is_chain_block, commitment }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Hash {
        Hash::from_slice(&[byte; 32])
    }

    #[test]
    fn classify_pending_when_off_chain() {
        let c = BlockCommitment::classify(h(1), 100, 1000, false, 432_000);
        assert_eq!(c.commitment, CommitmentLevel::Pending);
        // Confirmations are still computed even for off-chain blocks
        // (informational; UX may want to display them).
        assert_eq!(c.confirmations, 900);
        assert!(!c.is_chain_block);
    }

    #[test]
    fn classify_accepted_when_chain_low_confirmations() {
        let c = BlockCommitment::classify(h(1), 950, 1000, true, 432_000);
        assert_eq!(c.commitment, CommitmentLevel::Accepted);
        assert_eq!(c.confirmations, 50);
    }

    #[test]
    fn classify_confirmed_at_threshold_exactly() {
        let c = BlockCommitment::classify(h(1), 0, CONFIRMED_DEPTH_BLOCKS, true, 432_000);
        assert_eq!(c.commitment, CommitmentLevel::Confirmed);
        assert_eq!(c.confirmations, CONFIRMED_DEPTH_BLOCKS);
    }

    #[test]
    fn classify_one_below_confirmed_is_accepted() {
        let c = BlockCommitment::classify(h(1), 1, CONFIRMED_DEPTH_BLOCKS, true, 432_000);
        assert_eq!(c.commitment, CommitmentLevel::Accepted);
        assert_eq!(c.confirmations, CONFIRMED_DEPTH_BLOCKS - 1);
    }

    #[test]
    fn classify_finalized_at_threshold_exactly() {
        let c = BlockCommitment::classify(h(1), 0, 432_000, true, 432_000);
        assert_eq!(c.commitment, CommitmentLevel::Finalized);
    }

    #[test]
    fn classify_one_below_finalized_is_confirmed() {
        let c = BlockCommitment::classify(h(1), 1, 432_000, true, 432_000);
        assert_eq!(c.commitment, CommitmentLevel::Confirmed);
        assert_eq!(c.confirmations, 432_000 - 1);
    }

    #[test]
    fn classify_block_at_sink_is_accepted_with_zero_confirmations() {
        let c = BlockCommitment::classify(h(1), 1000, 1000, true, 432_000);
        assert_eq!(c.commitment, CommitmentLevel::Accepted);
        assert_eq!(c.confirmations, 0);
    }

    #[test]
    fn classify_saturating_sub_does_not_panic_on_future_block() {
        // Defensive: a block whose blue_score somehow exceeds current
        // (shouldn't happen but might during reorg races) gets 0
        // confirmations rather than a panic.
        let c = BlockCommitment::classify(h(1), 2000, 1000, true, 432_000);
        assert_eq!(c.confirmations, 0);
        assert_eq!(c.commitment, CommitmentLevel::Accepted);
    }

    #[test]
    fn confirmed_depth_constant_matches_design() {
        assert_eq!(CONFIRMED_DEPTH_BLOCKS, 100);
    }

    #[test]
    fn level_byte_assignment_is_frozen() {
        assert_eq!(CommitmentLevel::Pending.as_u8(), 0);
        assert_eq!(CommitmentLevel::Accepted.as_u8(), 1);
        assert_eq!(CommitmentLevel::Confirmed.as_u8(), 2);
        assert_eq!(CommitmentLevel::Finalized.as_u8(), 3);
    }
}
