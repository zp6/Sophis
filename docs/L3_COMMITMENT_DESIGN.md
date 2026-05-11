# L3 — Block Commitment Levels RPC

> **Status:** design frozen for sub-fase L3.0 — ready for L3.1 implementation.
> **Originating roadmap:** Roadmap L (Solana lessons), item L3.
> **Companion docs:** future `docs/L3_RUNBOOK.md` (deferred follow-up)
> and `SIPS/SIP-5-COMMITMENT.md` (also deferred follow-up).
> **Pre-existing baseline:** confirmation counts are exposed today on
> `getDaPayloadStatus` (Phase 6 DA) and as a `min_confirmation_count`
> filter on `getVirtualChainFromBlock`. Both are payload- or chain-walk
> centric — there is no per-block "what's the commitment level of this
> specific block?" RPC. L3 adds it.

## 1. Motivation

Every production smart-contract platform that exposes both pre-finality
and post-finality state has a **commitment-level** RPC abstraction. The
pattern lets clients pick their own latency-vs-safety tradeoff by
asking "is this block at level X?" rather than computing
"sink_blue_score - block_blue_score >= N" themselves.

- **Solana** has `getSignatureStatuses` returning `confirmationStatus
  ∈ {processed, confirmed, finalized}`.
- **Ethereum** has `eth_getBlockByNumber` accepting `latest`,
  `safe`, `finalized` block tags (post-Merge).
- **Sophis** has *just enough plumbing* in the existing `getSink` +
  `getBlock` calls for clients to compute the same thing manually,
  but no canonical RPC. dApps each invent their own threshold.

Without a canonical RPC:

- **dApps disagree on what "confirmed" means.** Some treat 1
  confirmation as confirmed; others wait 100. Indexers see
  inconsistent UX across wallets.
- **Wallet status pages cannot show meaningful state** beyond
  "submitted / accepted" without writing their own polling logic.
- **Bridge integrators have to track finality_depth themselves**, which
  requires reading per-network params and doing the math at every
  query.

L3 solves the *structural* problem with a single read-only RPC method
that returns a per-block commitment level computed against the local
node's view of the chain. Wallet UX (which level to require for which
operation) is downstream policy — L3 is just the read primitive.

This is a P1 deliverable per the original Roadmap L priorities (see
`project_solana_lessons.md`) and an ideal candidate for SIP-5 in the
follow-up.

## 2. Ratified design decisions

These decisions were committed by the founder on 2026-05-10 and are
frozen for the L3 implementation. Re-opening any of them requires a
new SIP.

| ID | Question | Choice | Rationale |
|----|----------|--------|-----------|
| **D1** | Commitment level enum | `Pending` / `Accepted` / `Confirmed` / `Finalized` | 4 levels covers every observed UX pattern. `Pending` (block exists but is off the selected chain) gives wallets a way to show "your tx may still be re-orged" rather than just disappearing. `Accepted` is on-chain but young. `Confirmed` and `Finalized` are the two "stable" thresholds. Ethereum's `safe` tag is folded into `Confirmed`; Solana's `processed` is folded into `Accepted`. Adding more levels (e.g. `Justified`) would require a PoS finality gadget which is explicitly out of scope (`project_finality_decisao.md`). |
| **D2** | `confirmed_depth` value | 100 blocks (= ~10 seconds @ 10 BPS) | A confirmed block is ~10 seconds old in wall-clock terms — short enough that real-time UX (DEX swaps, NFT mints) feels responsive, long enough that the probability of reorg is already low. Mirrors the Bitcoin "6 confirmations" idiom (~60 minutes) but at Sophis BPS scaled. Per-network override via `Params`; defaults to 100 on every preset. Calibrate post-devnet if measurements suggest a different sweet spot. |
| **D3** | `finalized_depth` value | `params.finality_depth` (= 432 000 blocks @ 10 BPS / 12 h) | Reuses the existing consensus finality_depth — no new constant, no new tunable. A finalized block in L3 means the same thing it means everywhere else in Sophis: past the chain's finality horizon, reorg requires a 12 h fork race that has never happened on any RandomX chain. |
| **D4** | Query parameter | `block_hash` (caller knows the block) | `tx_id → block` reverse-lookup index does not exist in Sophis today; building one would be a substantial new RocksDB store. Caller-supplied `block_hash` defers that work to clients (who get the hash from `submitTransaction` or `getVirtualChainNotification`). A future SIP could add a `getCommitmentByTxId` if a real use case justifies the index. |
| **D5** | `Pending` semantics | Block hash is known to the node but not on the selected chain | A block can be in the database (`statuses_store` returns `StatusUTXOValid`) but excluded from the GHOSTDAG selected chain. From the wallet's POV that's a "soft reject" — the tx hasn't been rolled back on consensus, but it hasn't gone canonical either. `Pending` distinguishes this from "unknown block" (which returns `None`). |

## 3. Wire format

### 3.1 Request

```text
GetBlockCommitmentRequest {
    block_hash: RpcHash,   // 32-byte block hash
}
```

### 3.2 Response

```text
GetBlockCommitmentResponse {
    commitment: Option<RpcBlockCommitment>,
}

RpcBlockCommitment {
    block_hash:          RpcHash,    // echoed
    block_blue_score:    u64,        // GHOSTDAG blue score of the block
    current_blue_score:  u64,        // sink blue score at query time
    confirmations:       u64,        // saturating_sub(current, block); 0 if block is at sink
    is_chain_block:      bool,       // is `block_hash` on the GHOSTDAG selected chain?
    commitment:          RpcCommitmentLevel,
}

RpcCommitmentLevel = Pending | Accepted | Confirmed | Finalized
```

`commitment` is `None` when `block_hash` is unknown to the node (block
not in DB, or pruned).

### 3.3 Commitment level mapping

```text
match (is_chain_block, confirmations) {
    (false, _)                                 => Pending,
    (true, c) if c >= params.finality_depth   => Finalized,
    (true, c) if c >= CONFIRMED_DEPTH_BLOCKS  => Confirmed,
    (true, _)                                 => Accepted,
}
```

`CONFIRMED_DEPTH_BLOCKS = 100` is a constant in `consensus-core`; can
be promoted to a per-network parameter in a future SIP if measurements
justify it.

## 4. Threat model

| ID | Threat | Mitigation |
|----|--------|------------|
| T1 | Eclipse: adversary serves a forged chain to the victim, who sees a "finalized" block that the rest of the network does not | Out of scope for L3. Eclipse defenses are at the P2P / sync layer. A victim under eclipse sees forged commitment levels for the same reason they see forged everything-else. |
| T2 | Race: client queries during a reorg and gets a "Confirmed" answer for a block that gets demoted to "Pending" 100 ms later | Acceptable. The L3 contract is "this is what the node sees right now"; clients that need stronger guarantees use `Finalized` (which by construction cannot be demoted in any realistic time window). |
| T3 | Wallet defaults to `Accepted` for high-value operations | Wallet UX policy, not L3's problem. The runbook will recommend `Confirmed` for routine UX and `Finalized` for bridge / withdrawal paths. |
| T4 | dApp interprets `Pending` as "rejected" and re-submits, double-spending | Wallet UX issue. `Pending` literally means "block exists, off chain right now" — could go either way on the next chain reorganisation. Re-submission with the same nonce/inputs is rejected by mempool dedupe. |
| T5 | Pruning races: block was finalized 13 hours ago, now pruned; query returns `None` | Documented behaviour. After pruning the commitment level is irrecoverable; clients that need historical commitment proofs MUST cache them client-side. Archive nodes (deferred per Roadmap J8) preserve the data. |
| T6 | Client polls `getBlockCommitment` aggressively, hammering the node | Per-call cost is two O(1) RocksDB reads (statuses_store + selected_chain_store) plus two ghostdag_store lookups. Cheap. Operators can rate-limit at the reverse-proxy layer if abuse appears; no built-in throttling. |

## 5. Comparison vs alternatives

| System | Levels | Per-network tunable | Off-chain "Pending" state | Per-block API | Bridge-grade finality |
|--------|--------|---------------------|----------------------------|----------------|------------------------|
| Solana `getSignatureStatuses` | processed/confirmed/finalized | no | no (sigs map to slots) | no (per-sig) | yes (rooted) |
| Ethereum block tags | latest/safe/finalized | no | implicit (pre-`safe` = pending) | yes (block-by-number / block-by-hash) | yes (post-Merge) |
| **Sophis L3** | Pending/Accepted/Confirmed/Finalized | thresholds in code, future SIP can promote | yes (explicit level) | yes (block-by-hash) | yes (probabilistic, finality_depth) |

Sophis L3 differs from Solana by exposing the off-chain state explicitly
(`Pending`) and from Ethereum by drawing a separate line between
"recent" (`Accepted`) and "comfortable" (`Confirmed`).

## 6. Out-of-scope (for L3)

The following are deliberately deferred:

- **`getCommitmentByTxId`** — requires a `tx_id → block` reverse index;
  not built in v1.
- **WebSocket subscription** — pull-only in v1; clients that need
  push-mode commitment updates can subscribe to the existing
  `BlockAddedNotification` and re-query.
- **Per-chain configurable thresholds** — `CONFIRMED_DEPTH_BLOCKS = 100`
  is a constant; promotable to per-network `Params` in a future SIP.
- **Bridge-specific levels** (e.g. `BridgeSafe`) — bridges out-of-scope
  per Decisão 4 of the 2026-05-04 pivot.

## 7. Frozen ABI surface

The following are **frozen** as of the L3 implementation merge. Any
change requires a hard fork.

| Item | Value |
|------|-------|
| `RpcApiOps::GetBlockCommitment` | `158` |
| Method name (gRPC) | `GetBlockCommitment` |
| Method name (wRPC JSON) | `getBlockCommitment` |
| `CONFIRMED_DEPTH_BLOCKS` | `100` (in `consensus-core`) |
| Commitment levels | `Pending=0`, `Accepted=1`, `Confirmed=2`, `Finalized=3` (u8 wire) |
| gRPC oneof slots | request 1128, response 1129 |

## 8. Reference implementation map

| Sub-fase | Scope |
|---------|-------|
| L3.0 | This design document |
| L3.1 | `rpc-core::model::commitment` types + ops enum + RpcApi trait method |
| L3.2 | `ConsensusApi::get_block_commitment` default + Consensus impl + session wrapper |
| L3.3 | `RpcCoreService::get_block_commitment_call` + 2 mock stubs |
| L3.4 | gRPC binding (proto + ops + conversions + route + factory) + wRPC binding (server router + client macro list) + integration test |
| L3.5 | Workspace check + clippy strict + single commit |

## 9. Glossary

| Term | Meaning |
|------|---------|
| Commitment level | Discrete classification of a block's relationship to the GHOSTDAG selected chain plus its depth from the sink. One of `Pending`, `Accepted`, `Confirmed`, `Finalized`. |
| Selected chain | The GHOSTDAG canonical "main chain" projection of the DAG. A block is on it iff `selected_chain_store.get_by_hash(block_hash)` returns `Ok`. |
| `confirmations` | `current_blue_score - block_blue_score` (saturating). Sink-relative. |
| `Pending` | Block exists in the node's database but is not on the selected chain. May join the chain on the next reorg, or remain off forever. |
| `Accepted` | Block is on the selected chain but `confirmations < CONFIRMED_DEPTH_BLOCKS`. |
| `Confirmed` | Block is on the selected chain with `confirmations ≥ 100` (≈ 10 s @ 10 BPS). |
| `Finalized` | Block is on the selected chain with `confirmations ≥ params.finality_depth` (≈ 12 h @ 10 BPS / 432 000 blocks). |
