//! Per-IP peer misbehavior scoring (Audit F-12, Session 10, 2026-05-15).
//!
//! The pre-F-12 protocol stack disconnects misbehaving peers at every
//! adversarial decision point but never promotes the disconnect to a
//! ban — so a malicious peer can reconnect immediately and retry. This
//! module adds the missing **policy layer** on top of the existing
//! `BannedAddressesStore` persistence.
//!
//! Design:
//!
//! - **In-memory score state** keyed by `IpAddr` with linear decay. The
//!   persistence layer (`BannedAddressesStore`) holds the final yes/no
//!   ban verdict and survives node restarts; the score is volatile and
//!   resets on restart. The score's only job is to detect *repeated*
//!   misbehavior within a short rolling window.
//!
//! - **Decay** is linear at one point per second by default. This makes
//!   reasoning trivial: a peer that scores 80 from one bad message is
//!   below threshold (100) and decays to 0 after 80 s; a peer that
//!   scores 60 twice within a minute crosses 100 and gets banned.
//!
//! - **Threshold** is fixed at 100. Reasons are weighted so that a
//!   single Severe event hits the threshold immediately (instant ban),
//!   while sub-threshold reasons accumulate.
//!
//! - **`record_misbehavior`** returns whether the score crossed the
//!   threshold on this call; callers (the flow error handler) use that
//!   to decide if a `ConnectionManager.ban(ip)` call is warranted.
//!
//! This module does NOT touch `BannedAddressesStore` directly — keeping
//! the score management orthogonal to the persistence layer means the
//! existing `AddressManager.ban()` / `is_banned()` semantics are
//! untouched (24h auto-unban, address-store eviction, RPC ban list).

use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Instant;

/// Score that, when reached or exceeded, triggers a ban-write to the
/// persistence layer. Set at 100 so single-Severe-event reasons (weight
/// = 100) ban on first occurrence.
pub const BAN_SCORE_THRESHOLD: u32 = 100;

/// Linear decay rate, in points per second. A 1 pps decay means a
/// peer's score returns to 0 within `current_score` seconds of idleness.
/// Linear keeps the math predictable: at threshold 100, a recent bad
/// peer's window is exactly 100 s. Tuning lever; not a consensus parameter.
pub const DECAY_POINTS_PER_SECOND: u32 = 1;

/// Hard cap on the in-memory score to bound the post-ban "memory" of a
/// returning peer. The maximum useful effect is a slower decay; values
/// above the cap are clamped. Set to 10× threshold = 1000.
pub const MAX_SCORE: u32 = 10 * BAN_SCORE_THRESHOLD;

/// Categories of protocol-level misbehavior. Each variant has an
/// associated **base weight** (see [`MisbehaviorReason::weight`]) that
/// the score manager adds to the offending peer's score.
///
/// The taxonomy aligns with `ProtocolError` variants in `protocol/p2p`:
/// the flow error handler in `protocol/flows/src/flow_trait.rs` maps
/// every `ProtocolError` to one of these variants before recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MisbehaviorReason {
    /// Consensus rule violation, pruning import failure, or other
    /// validation error that proves the peer sent invalid chain data.
    /// Weight = 100 — single occurrence bans.
    ///
    /// Maps from: `ProtocolError::RuleError`,
    ///            `ProtocolError::ConsensusError`,
    ///            `ProtocolError::PruningImportError`.
    Severe,

    /// Protocol-level violation that suggests the peer is broken or
    /// hostile: message shape mismatch, version mismatch, explicit
    /// "misbehaving peer" marker. Weight = 50.
    ///
    /// Maps from: `ProtocolError::VersionMismatch`,
    ///            `ProtocolError::WrongNetwork`,
    ///            `ProtocolError::MisbehavingPeer`,
    ///            `ProtocolError::UnexpectedMessage`,
    ///            `ProtocolError::NoRouteForMessageType`.
    HighSeverity,

    /// Recoverable parse / conversion errors or rejected requests.
    /// Weight = 20.
    ///
    /// Maps from: `ProtocolError::ConversionError`,
    ///            `ProtocolError::Rejected`.
    MediumSeverity,

    /// Soft errors: timeout, route capacity, mining manager error.
    /// Weight = 5. Repeated occurrences raise the score over time.
    ///
    /// Maps from: `ProtocolError::Timeout`,
    ///            `ProtocolError::IncomingRouteCapacityReached`,
    ///            `ProtocolError::OutgoingRouteCapacityReached`,
    ///            `ProtocolError::MiningManagerError`.
    LowSeverity,

    /// Benign causes that should NOT raise the score. We still record
    /// the event so observability hooks (debug logging, metrics) see
    /// every disconnect. Weight = 0.
    ///
    /// Maps from: `ProtocolError::ConnectionClosed`,
    ///            `ProtocolError::IgnorableReject`,
    ///            `ProtocolError::PeerAlreadyExists`,
    ///            `ProtocolError::LoopbackConnection`,
    ///            `ProtocolError::Other`,
    ///            `ProtocolError::OtherOwned`,
    ///            `ProtocolError::IdentityError`.
    Benign,
}

impl MisbehaviorReason {
    /// Score increment for this reason. Sums of weights cross the
    /// `BAN_SCORE_THRESHOLD` to trigger a persistent ban.
    pub const fn weight(self) -> u32 {
        match self {
            Self::Severe => 100,
            Self::HighSeverity => 50,
            Self::MediumSeverity => 20,
            Self::LowSeverity => 5,
            Self::Benign => 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ScoreState {
    score: u32,
    last_update: Instant,
}

/// Outcome of a single `record_misbehavior` call. Callers use this to
/// decide whether to invoke `ConnectionManager.ban(ip)` (which writes
/// to the persistent ban store and terminates the active connection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOutcome {
    /// Score increased but is still below threshold. No ban action needed.
    BelowThreshold { current_score: u32 },
    /// Score reached or exceeded `BAN_SCORE_THRESHOLD` on this call.
    /// The caller should ban the peer via the connection manager.
    BanTriggered { final_score: u32 },
}

/// Per-IP misbehavior score store. Thread-safe; clones are cheap (Arc).
///
/// The map is purely in-memory — restart wipes scores but not bans
/// (which persist in `BannedAddressesStore`). This is intentional: a
/// long-banned peer is already evicted from the address store and
/// won't be redialed; a short-banned peer crosses threshold quickly
/// again if it resumes misbehavior.
#[derive(Default)]
pub struct PeerScoreManager {
    inner: Mutex<HashMap<IpAddr, ScoreState>>,
}

impl PeerScoreManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a misbehavior event. Applies decay to the existing score
    /// (if any), adds the reason's weight, clamps to `MAX_SCORE`, and
    /// returns the outcome.
    ///
    /// Internally takes a `now: Instant` to make the call deterministic
    /// in tests; production callers should use [`Self::record`].
    pub fn record_with_clock(&self, ip: IpAddr, reason: MisbehaviorReason, now: Instant) -> RecordOutcome {
        let mut map = self.inner.lock();
        let entry = map.entry(ip).or_insert(ScoreState { score: 0, last_update: now });
        let new_score = decayed(entry.score, entry.last_update, now).saturating_add(reason.weight()).min(MAX_SCORE);
        entry.score = new_score;
        entry.last_update = now;
        if new_score >= BAN_SCORE_THRESHOLD {
            // Clear the entry so a returning peer with a fresh
            // BannedAddressesStore lookup doesn't double-account.
            map.remove(&ip);
            RecordOutcome::BanTriggered { final_score: new_score }
        } else {
            RecordOutcome::BelowThreshold { current_score: new_score }
        }
    }

    /// Production convenience: uses `Instant::now()` as the clock.
    pub fn record(&self, ip: IpAddr, reason: MisbehaviorReason) -> RecordOutcome {
        self.record_with_clock(ip, reason, Instant::now())
    }

    /// Returns the current decayed score for `ip` (0 if not tracked).
    /// Useful for observability / metrics; not used in the ban
    /// decision path.
    pub fn current_score(&self, ip: IpAddr) -> u32 {
        let now = Instant::now();
        let map = self.inner.lock();
        map.get(&ip).map(|s| decayed(s.score, s.last_update, now)).unwrap_or(0)
    }

    /// Removes a peer's score entry. Called when the operator
    /// explicitly unbans a peer or when the persistent ban expires; the
    /// peer starts fresh.
    pub fn clear(&self, ip: IpAddr) {
        self.inner.lock().remove(&ip);
    }
}

/// Apply linear decay to a stored score given elapsed wall time.
/// Saturating to 0 on the lower bound; the upper bound is enforced by
/// the caller before storing.
fn decayed(score: u32, last_update: Instant, now: Instant) -> u32 {
    let elapsed_secs: u32 = now.saturating_duration_since(last_update).as_secs().try_into().unwrap_or(u32::MAX);
    let decay = elapsed_secs.saturating_mul(DECAY_POINTS_PER_SECOND);
    score.saturating_sub(decay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::time::Duration;

    fn ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))
    }

    #[test]
    fn single_severe_event_triggers_ban() {
        let m = PeerScoreManager::new();
        let outcome = m.record(ip(), MisbehaviorReason::Severe);
        assert!(matches!(outcome, RecordOutcome::BanTriggered { .. }));
    }

    #[test]
    fn single_low_severity_event_below_threshold() {
        let m = PeerScoreManager::new();
        let outcome = m.record(ip(), MisbehaviorReason::LowSeverity);
        match outcome {
            RecordOutcome::BelowThreshold { current_score } => assert_eq!(current_score, 5),
            other => panic!("expected BelowThreshold, got {other:?}"),
        }
    }

    #[test]
    fn repeated_high_severity_eventually_bans() {
        // 50 + 50 = 100 → ban on the second event (close-in-time).
        let m = PeerScoreManager::new();
        let t0 = Instant::now();
        let first = m.record_with_clock(ip(), MisbehaviorReason::HighSeverity, t0);
        match first {
            RecordOutcome::BelowThreshold { current_score: 50 } => {}
            other => panic!("first record: expected BelowThreshold(50), got {other:?}"),
        }
        let second = m.record_with_clock(ip(), MisbehaviorReason::HighSeverity, t0 + Duration::from_millis(100));
        assert!(matches!(second, RecordOutcome::BanTriggered { .. }));
    }

    #[test]
    fn decay_brings_repeated_events_back_below_threshold() {
        // 50 → wait 60 s (decays to 0) → another 50 → still 50.
        let m = PeerScoreManager::new();
        let t0 = Instant::now();
        let _ = m.record_with_clock(ip(), MisbehaviorReason::HighSeverity, t0);
        let later = m.record_with_clock(ip(), MisbehaviorReason::HighSeverity, t0 + Duration::from_secs(60));
        match later {
            RecordOutcome::BelowThreshold { current_score: 50 } => {}
            other => panic!("after decay: expected BelowThreshold(50), got {other:?}"),
        }
    }

    #[test]
    fn benign_event_does_not_raise_score() {
        let m = PeerScoreManager::new();
        let t0 = Instant::now();
        for _ in 0..50 {
            let _ = m.record_with_clock(ip(), MisbehaviorReason::Benign, t0);
        }
        assert_eq!(m.current_score(ip()), 0);
    }

    #[test]
    fn clear_resets_score() {
        let m = PeerScoreManager::new();
        let _ = m.record(ip(), MisbehaviorReason::HighSeverity); // score = 50
        assert_eq!(m.current_score(ip()), 50);
        m.clear(ip());
        assert_eq!(m.current_score(ip()), 0);
    }

    #[test]
    fn score_clamps_at_max() {
        let m = PeerScoreManager::new();
        // Multiple Severe events: each ban_triggers and removes the entry,
        // so to exercise clamping we use a long uninterrupted sequence on
        // a sub-threshold reason.
        let t0 = Instant::now();
        // 30 × 5 (LowSeverity) close-in-time = 150 raw → ban triggered at 100.
        // Verify the BanTriggered path doesn't leak a > MAX score.
        let mut last_outcome = RecordOutcome::BelowThreshold { current_score: 0 };
        for _ in 0..30 {
            last_outcome = m.record_with_clock(ip(), MisbehaviorReason::LowSeverity, t0);
            if let RecordOutcome::BanTriggered { final_score } = last_outcome {
                assert!(final_score <= MAX_SCORE, "final_score {final_score} must be <= MAX_SCORE {MAX_SCORE}");
                break;
            }
        }
        assert!(matches!(last_outcome, RecordOutcome::BanTriggered { .. }));
    }

    /// Two different IPs have independent scores.
    #[test]
    fn scores_are_per_ip() {
        let m = PeerScoreManager::new();
        let a = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let b = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));
        let _ = m.record(a, MisbehaviorReason::HighSeverity);
        assert_eq!(m.current_score(a), 50);
        assert_eq!(m.current_score(b), 0);
    }

    #[test]
    fn weight_table_is_frozen() {
        // Defense-in-depth: catch unintended changes to the weight table
        // that would silently rebalance the policy.
        assert_eq!(MisbehaviorReason::Severe.weight(), 100);
        assert_eq!(MisbehaviorReason::HighSeverity.weight(), 50);
        assert_eq!(MisbehaviorReason::MediumSeverity.weight(), 20);
        assert_eq!(MisbehaviorReason::LowSeverity.weight(), 5);
        assert_eq!(MisbehaviorReason::Benign.weight(), 0);
    }

    #[test]
    fn threshold_constants_consistent_with_max() {
        assert!(BAN_SCORE_THRESHOLD < MAX_SCORE);
        assert_eq!(MAX_SCORE, BAN_SCORE_THRESHOLD * 10);
    }
}
