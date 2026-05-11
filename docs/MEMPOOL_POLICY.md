# Sophis Mempool Policy

**Status:** v1, drafted 2026-05-09. Reference document for wallet
authors, exchanges, and node operators. Describes mempool admission
rules, replace-by-fee (RBF) semantics, fee/mass model, and the status
of related Bitcoin-style policies (CPFP, package relay).

This document is **descriptive, not prescriptive**. It describes
what Sophis 1.1 does today; consensus and policy may evolve via SIP
post-mainnet (`HARD_FORK_POLICY.md`).

---

## 1. Mempool basics

The mempool is implemented in `mining/src/mempool/`. Public types and
behavior live in `mining/src/mempool/mod.rs`; internal model in
`mining/src/mempool/model/`.

| Property | Value / behavior |
|---|---|
| Eviction policy | Lowest-feerate-first when capacity is exceeded |
| Mass accounting | Per-transaction `mass` field (consensus-defined); fees normalized as `fee / mass` (units: sompi/gram) |
| Orphan pool | Separate pool for transactions whose inputs are not yet visible (`mining/src/mempool/model/orphan_pool.rs`) |
| Standardness check | `mining/src/mempool/check_transaction_standard.rs` enforces script standardness, output dust thresholds, and signature-scheme conformance (Dilithium-only) |
| Frontier / selector | `mining/src/mempool/model/frontier/` provides the search tree used by `block_template/selector` for inclusion-priority ordering |

A transaction admitted to the mempool is not guaranteed to be
included in any particular future block — block templates are
selected from the frontier based on feerate, mass, dependencies, and
DAG state.

## 2. Replace-by-fee (RBF)

Sophis supports RBF with three explicit policies, defined in
`mining/src/mempool/model/tx.rs` (`RbfPolicy`) and dispatched in
`mining/src/mempool/replace_by_fee.rs`.

### 2.1 `RbfPolicy::Forbidden`

The default policy for transactions submitted via RPC without an
explicit RBF flag. A double-spend is rejected outright.

### 2.2 `RbfPolicy::Allowed`

The transaction may either be admitted alongside non-conflicting
predecessors **or** replace conflicting predecessors if its feerate
exceeds the maximum feerate among all double-spent transactions.

This is the closest analog to Bitcoin BIP-125 opt-in RBF, but it is
expressed at the **policy layer**, not via a per-input flag — there
is no `nSequence`-equivalent signaling on Sophis. The submitter
indicates RBF intent through the RPC submission policy.

### 2.3 `RbfPolicy::Mandatory`

The submission **must** replace exactly one conflicting transaction.
If zero or more than one double-spend is found, submission fails
(`RejectRbfNoDoubleSpend` / `RejectRbfTooManyDoubleSpendingTransactions`).

This policy is intended for tools that explicitly want to bump fees
on a known-pending transaction (fee-bumping wallets, exchange
acceleration scripts).

### 2.4 RBF feerate threshold

For `Allowed` and `Mandatory` policies, the inbound transaction must
satisfy `feerate(new) > max(feerate(replaced_i))`. The threshold is
computed in `Mempool::get_replace_by_fee_constraint`. There is no
absolute-fee bump requirement (unlike Bitcoin BIP-125 rule 4) — the
network enforces only the feerate inequality.

### 2.5 RBF and rapid block production

Sophis targets 10 blocks per second. The RBF window is therefore
much shorter than on Bitcoin in absolute time. Practitioners should
treat RBF as a "very-recent submission" remediation, not a
multi-minute fee-bumping flow.

## 3. CPFP (child-pays-for-parent) — not implemented

Sophis 1.1 does **not** implement CPFP-style mempool selection.
Specifically:

- The block-template selector does not bump a low-feerate parent's
  inclusion priority based on a high-feerate child
- `mining/src/mempool/model/frontier/selectors.rs` orders by
  per-transaction feerate without considering descendant feerate

**Implication:** if a low-feerate transaction is stuck in the
mempool, attaching a high-feerate child does not currently help it
get mined. The remedy is to RBF the parent directly.

CPFP is on the post-mainnet research backlog (Roadmap K4 covers the
audit + documentation; an explicit CPFP implementation would be a
separate follow-up SIP).

## 4. Package relay — not implemented

Bitcoin Core's package relay (BIP 331) lets a node accept a parent
+ child pair atomically when neither would be admitted alone. Sophis
1.1 does not implement package relay. A child whose parent is not in
the mempool goes to the orphan pool and is reconsidered when the
parent arrives.

For high-mass dependent transactions, submitters should:

1. Submit the parent first
2. Wait for mempool admission (verify via RPC)
3. Submit the child

This is acceptable at 10 BPS because mempool propagation is sub-second
in healthy network conditions.

## 5. Standardness rules

`mining/src/mempool/check_transaction_standard.rs` enforces:

- **Signature scheme**: Dilithium ML-DSA-44 only. Any transaction
  whose unlock scripts attempt secp256k1, Schnorr, ed25519, or any
  other classical signature primitive is rejected at admission.
- **Script size limits**: per-input and per-output script size caps;
  see the constants in the same file.
- **Output dust thresholds**: outputs below a network-defined dust
  amount are rejected at admission. Wallets should verify this floor
  via RPC `get_dust_threshold` (or equivalent — see RPC reference) for
  the active network.
- **Mass limit per tx**: enforced both at admission and at block
  template construction; over-mass transactions are rejected
  outright.

## 6. Fee / mass model

Sophis fees are denominated in **sompi per gram** (`fee / mass`),
where:

- `1 SPHS = 100,000,000 sompi` (8 decimals)
- `mass` is the consensus-defined transaction weight, accounting for
  signature size, output count, and computation cost
- Dilithium signatures are ~2.5 KB each — much larger than secp256k1.
  This is reflected in mass, and therefore in the absolute fee a
  signed transaction pays at any given feerate

`mining/src/feerate/mod.rs` provides `FeerateEstimations` in three
buckets:

| Bucket | Inclusion target |
|---|---|
| `priority_bucket` | sub-second DAG inclusion |
| `normal_buckets` | sub-minute DAG inclusion |
| `low_buckets` | sub-hour DAG inclusion |

See `docs/FEE_PRIORITY.md` for the priority-fee API reference.

## 7. Guidance for wallets

| Use case | Recommendation |
|---|---|
| Standard payment | Use `RbfPolicy::Forbidden` (default). Stable submission, no replacement risk |
| User-bumpable wallet | Use `RbfPolicy::Allowed` and surface "speed up" UX. Submitter increases feerate; client re-signs and re-submits |
| Fee-acceleration tool | Use `RbfPolicy::Mandatory`. Fails fast if no double-spend found, which is the desired UX for "I know exactly which tx I'm bumping" |
| Multi-step protocol (parent → child) | Submit parent first, await mempool admission, then submit child. Do not rely on package relay |
| High-value / time-sensitive | Use the `priority_bucket` feerate; bump via RBF if not included within ~1s |

## 8. Guidance for exchanges

- **Confirmation policy**: Sophis is probabilistic. Use the multi-level
  commitment exposure documented in the RPC reference (`accepted` →
  `confirmed` → `finalized`) — see Roadmap L3.
- **RBF awareness**: a deposit transaction that is `accepted` may
  still be replaced by a higher-fee variant before it is `confirmed`.
  Wait for `finalized` before crediting the customer's account on the
  exchange ledger.
- **Withdrawal acceleration**: implement RBF with `Mandatory` policy
  for stuck withdrawals; surface the new tx hash to the customer.

## 9. Guidance for node operators

- **Mempool size limit**: configurable in `mining/src/mempool/config.rs`.
  Adjust based on RAM headroom; default is conservative.
- **Orphan pool TTL**: orphans expire after a configurable interval;
  do not rely on indefinite retention.
- **Standardness vs consensus**: standardness is a policy layer.
  A non-standard but consensus-valid transaction may be admitted by a
  permissive node and mined. Operators choosing to relax standardness
  should be aware that this affects DOS exposure.

## 10. What this is NOT

- This document is not a SIP. It describes the **current** behavior
  of Sophis 1.1.
- This document does not describe consensus rules. Consensus is in
  `consensus/`; mempool policy is a relay/admission concern.
- This document does not commit to never adding CPFP or package
  relay. Either may land via post-mainnet SIP if there is clear demand
  and the design fits the 10-BPS model.

## 11. Reference

- Code: `mining/src/mempool/`, `mining/src/feerate/`,
  `mining/src/manager.rs`
- Companion: `docs/FEE_PRIORITY.md`, `docs/SVM_EXECUTION_MODEL.md`
- RPC API: `rpc/core/`, `rpc/grpc/core/proto/rpc.proto`,
  `rpc/wrpc/server/`
- Cultural reference: Bitcoin Core mempool policy documentation
  (the design lineage), Kaspa rusty-kaspa (the implementation
  lineage); Sophis adapts both rather than re-deriving from scratch
