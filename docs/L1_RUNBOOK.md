# L1 ALT — Operator Runbook

> Companion to `docs/L1_ALT_DESIGN.md` and `SIPS/SIP-3-ALT.md`. This
> document is for **node operators** and **wallet implementors** who need
> to know how to interact with the L1 Address Lookup Table feature in
> day-to-day operation.

## Audience

* **Node operators** running `sophisd`: §1, §3, §6
* **Wallet developers** consuming v=1 transactions: §2, §4, §5
* **dApp developers** writing sVM contracts that resolve ALT references: §5
* **Bridge / exchange operators** ingesting Sophis transactions: §2, §4

## 1. Activation status

L1 ALT is **active at genesis on every Sophis network** (mainnet, testnet,
devnet, simnet). There is no soft-fork window, no flag day, and no
operator action required to "turn on" the feature.

* `TX_VERSION` = 0 (legacy default; v=0 transactions remain valid forever)
* `MAX_TX_VERSION` = 1 (highest accepted version; enables ALT outputs)
* `min_alt_activation_daa_score` = 0 on every network

A node compiled from `sophis-network/Sophis@c009b99` or later automatically
indexes ALT-creation outputs into the local `DbAltStore` as part of every
chain-block commit.

## 2. Recognising ALT in v=1 transactions

A v=1 transaction output's `script_public_key.script()` falls into one of
three categories based on the leading byte:

| `script[0]` | Category | Length | Validator behaviour |
|-------------|----------|--------|---------------------|
| `0x00`–`0x7F` | Inline `ScriptPublicKey` | variable | Standard verification |
| `0xFD` | ALT reference | exactly 8 bytes | Resolved via `alt_store` |
| `0xFE` | ALT-creation output | ≥ 22 bytes (header) + entries | Indexed into `alt_store` |
| `0x80`–`0xFC`, `0xFF` | Reserved | n/a | Rejected (no current rules) |

For v=0 transactions, the leading-byte interpretation is purely the legacy
ScriptPublicKey grammar; `0xFD` and `0xFE` are not interpreted as ALT
discriminators (rule 1 / 13 of DESIGN §5).

## 3. RocksDB store

Three new prefixes were allocated for L1 ALT, immediately after the
Phase 6 DA range:

| Prefix | Constant | Key | Value | Lifecycle |
|--------|----------|-----|-------|-----------|
| 200 | `AltEntries` | `handle: [u8; 6]` | `AltEntry` (handle + entries + creating block + DAA) | **Permanent** — never deleted |
| 201 | `AltCreatedInBlock` | `block_hash` | `AltBlockHandles` (Vec<handle>) | Pruned with the block |
| 202 | `AltHandleResolutions` | `handle: [u8; 6]` | `AltResolution` ((block_hash, daa_score)) | **Permanent** |

**Important:** prefix 200 and 202 grow monotonically. There is no pruning
policy that removes entries. The growth rate is bounded by the per-block
cap (16 ALT-creation outputs × ~10 BPS × max ~16 KB per ALT = ~2.5 MB per
day worst case under sustained adversarial spam, ~10× lower in practice
because of the 100 000-mass base cost).

Disk impact estimate over a year of operation, assuming saturation at the
cap: ~900 MB. Realistic estimate: ~50 MB. Either is well within typical
node disk budgets.

## 4. Wallet workflow (consumer side)

### 4.1 Resolving an ALT reference

When a wallet observes a v=1 transaction output whose `script[0] == 0xFD`,
it MUST resolve the reference before display:

```text
let r = parse_alt_reference(script)?;          // 14: bad length
let entry = rpc.get_alt_entry(r.handle).await?; // 15: dangling handle
let spk = entry.entries.get(r.index).ok_or(...)?; // 16: out of range
display_address(spk.spk_version, &spk.spk_script)
```

The three error cases map to consensus rules 14, 15, 16. Wallets should
display the underlying SPK using the same rules they apply to inline
v=0 outputs (e.g. P2PKH-Dilithium → bech32 address).

### 4.2 Building an ALT-using transaction

To send to a destination via an existing ALT entry rather than inline:

```text
let alt = rpc.get_alt_entry(known_handle).await?;
let idx = alt.entries.iter().position(|r| r.spk_script == target_spk)?;
let output = TransactionOutput {
    value: amount_sompi,
    script_public_key: ScriptPublicKey::new(0, encode_alt_reference_script(known_handle, idx as u8)),
};
let tx = Transaction::new(MAX_TX_VERSION, /* v=1 */ inputs, vec![output], ...);
```

Wallets MAY auto-detect when the same destination appears 2+ times in a
planned transaction and offer to create an ALT (cost amortizes after ~800
references for the conservative case; see DESIGN §6.3).

### 4.3 Creating a new ALT entry

ALT-creation outputs are unspendable (`value = 0`) and carry the entries
payload in their `script`. The transaction must be `version >= 1`:

```text
let script = encode_alt_creation_script(&[
    (0u16, /* spk_version */ &p2pkh_dilithium_template_a),
    (0u16, &p2pkh_dilithium_template_b),
])?;
let create_output = TransactionOutput {
    value: 0,
    script_public_key: ScriptPublicKey::new(0, script),
};
// Include create_output alongside normal change/payment outputs in a v=1 tx.
```

The handle is deterministic from the entries payload: any wallet that
encodes the same `(spk_version, spk_script)` sequence in the same order
gets the same handle. Race conditions between concurrent creators are
benign — the first to land in a block wins; the second creation becomes
a no-op at the store layer.

## 5. sVM contract integration

Contracts that need to interpret v=1 transaction outputs containing ALT
references must declare `Capability::ResolveAlt` in their manifest at
deploy time:

```text
ContractManifest::new(
    contract_id,
    UpgradePolicy::Immutable,
    vec![Capability::ResolveAlt, /* other caps */],
)
```

At runtime, contracts call the host function:

```text
extern "C" {
    /// Returns spk_version on hit (0..=u16::MAX), negative status on miss/error.
    fn sophis_alt_lookup(
        ptr_handle: *const u8,
        index: i32,
        out_ptr: *mut u8,
        out_len_ptr: *mut u32,
    ) -> i32;
}
```

Status codes are documented in
`svm/runtime/src/host.rs::sophis_alt_lookup` (search for the comment
block). Most error paths a well-behaved contract must handle:

- `-1`: capability not granted (manifest missing `ResolveAlt`)
- `-2`: gas exhausted
- `-4`: handle not found in registry
- `-5`: index out of range OR caller's buffer too small (retry with the
  size now in `out_len_ptr`)

Gas cost is `GAS_ALT_RESOLVE = 1500` per successful lookup, calibrated
post-devnet alongside `da_verify_cost`.

## 6. Node operator monitoring

### 6.1 Logs to watch

`sophisd` emits `WARN` lines on the rare paths where an ALT-creation
output passes consensus validation but fails to index (for example, if
the underlying RocksDB write batch fails):

```
ALT creation indexing failed for block <hash>: <error>
```

This log line should never appear in normal operation. If it does,
investigate the underlying RocksDB error before continuing — the chain
will keep advancing (the indexing failure is non-fatal by design, mirroring
the Phase 6 DA path), but downstream RPC consumers will see "dangling"
references for any output that failed to index.

### 6.2 RPC endpoints

The L1.6 sub-fase (operational follow-up) wires three RPC methods:

- `getAltEntry(handle: [u8; 6])` → full `AltEntry` (entries + creating block + DAA)
- `getAltResolution(handle: [u8; 6])` → lightweight `(block, DAA)` only
- `listAltsCreatedInBlock(block_hash)` → handles created in the given block

Until L1.6 lands, ALT data can be inspected directly from RocksDB using
the existing `sophis-database` CLI tooling (prefixes 200..202, decoded via
`AltEntry`/`AltResolution`/`AltBlockHandles` borsh).

### 6.3 Dashboards and alerts

Recommended metrics for operators running the dashboard at
`tools/sophis-dashboard/`:

- `alt_creations_per_block` — should rarely exceed 4-8 in normal traffic;
  sustained values near the cap of 16 indicate either heavy DEX/DAO
  activity or an adversary attempting to fill blocks
- `alt_entries_total` — monotonic counter; growth-rate alerts fire if
  rate exceeds 10 per minute sustained
- `alt_resolution_p99_latency` — RocksDB lookup latency; should stay
  sub-millisecond. Tail spikes suggest cache cold-misses (raise
  `block_data_cache_size`)

## 7. Pre-mainnet checklist

For operators planning to mine on mainnet day zero:

- [ ] Node binary is `sophis-network/Sophis@c009b99` or later (verify
      with `sophisd --version`).
- [ ] `target/release/sophisd.exe` includes the `alt` module (compile
      check: `cargo check -p sophis-consensus` should mention the alt
      crate).
- [ ] No custom mempool overrides for `DEFAULT_MAXIMUM_STANDARD_TRANSACTION_VERSION`
      below `MAX_TX_VERSION = 1`. Default mempool config is correct.
- [ ] Wallet software has been tested against v=1 transactions on devnet
      (use `dilithium-wallet` once L1.5 lands, or hand-roll until then).

## 8. Disengagement

L1 ALT cannot be cleanly removed once mainnet launches because:

1. ALT references in transactions become unverifiable without the registry.
2. Storage commitments include resolved-SPK bytes, so historical UTXO
   verification depends on the registry.

A future SIP that wished to deprecate ALT would have to either:

- Hard-fork the chain to disable v=1 acceptance (rejecting all in-flight
  ALT-using transactions), OR
- Soft-fork to freeze new ALT creations while preserving the existing
  registry forever (similar to how some Bitcoin opcodes were soft-disabled
  after BIP-66).

Operators considering forks of the codebase that strip ALT should follow
option (a) only and only at a fresh genesis.

## 9. References

- `docs/L1_ALT_DESIGN.md` — wire-format and consensus specification
- `SIPS/SIP-3-ALT.md` — SIP stub (full body deferred to post-testnet)
- `consensus/src/model/stores/alt.rs` — RocksDB store
- `consensus/src/svm_alt.rs` — sVM host backend
- `oracle/docs/PHASE6_RUNBOOK.md` — sibling runbook for Phase 6 DA;
  shares the same operational philosophy

## 10. Document history

| Date       | Change |
|------------|--------|
| 2026-05-10 | Initial runbook (sub-fase L1.8). |
