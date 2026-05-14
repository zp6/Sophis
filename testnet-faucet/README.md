# Sophis Testnet Faucet

HTTP faucet that drips SPHS to requesters on the Sophis testnet (or devnet/simnet).
Submits Dilithium-signed transactions through a local `sophisd` node.

> Testnet tokens have no monetary value. The faucet exists only to remove
> friction for developers building on Sophis.

## Architecture

```
                    HTTPS                  127.0.0.1:8181
   Internet ──→ nginx (TLS) ──→ testnet-faucet ──→ sophisd-testnet ──→ peers
                                                       │
                                                       └── /var/lib/sophis/testnet (RocksDB)
```

- Single static HTML page, no JavaScript framework, no external dependencies.
- Two endpoints: `GET /status` (JSON), `POST /drip` (JSON).
- Rate limit per **address** at the application layer (`--cooldown`).
- Rate limit per **IP** at the proxy layer (nginx + optional Cloudflare).
- Wallet is a single Dilithium ML-DSA-44 keypair stored as plaintext JSON
  (mode 0600). Never holds large balances — keep ≤ a few weeks of expected
  drip volume.

## Quick start (local devnet)

```bash
# 1. Build (requires Sophis build env — see CLAUDE.md)
cargo build -p testnet-faucet -p dilithium-wallet

# 2. Generate a faucet wallet
./target/debug/testnet-faucet generate-wallet \
    --wallet faucet_wallet.json \
    --network devnet

# 3. Start sophisd (assume devnet orchestrator is running on :46610)
#    Mine to the faucet address so it has UTXOs (wait coinbase maturity)
./target/debug/sophis-miner --rpcserver=localhost:46610 \
    --mining-address=<faucet-address>

# 4. Run the faucet
./target/debug/testnet-faucet start \
    --wallet faucet_wallet.json \
    --rpcserver localhost:46610 \
    --network devnet \
    --port 8181 \
    --amount 10000000 \
    --cooldown 60

# 5. Test
curl http://localhost:8181/status
curl -X POST http://localhost:8181/drip \
    -H 'content-type: application/json' \
    -d '{"address":"sophisdev:q..."}'
```

## Production deploy

Single-host deploy on Ubuntu 22.04/24.04 LTS, ~512 MB RAM, ~20 GB disk
(testnet chain ≈ 3–5 GB after months).

See [`deploy/`](deploy/) for:

| File                            | Purpose                                    |
|---------------------------------|--------------------------------------------|
| `deploy.sh`                     | Idempotent installer (run as sudo user)    |
| `sophisd-testnet.service`       | systemd unit for the node                  |
| `faucet.service`                | systemd unit for the faucet HTTP server    |
| `nginx-faucet.sophis.org.conf`  | Reverse proxy + TLS + rate limit           |
| `cloudflare-rules.md`           | Cloudflare WAF + bot mitigation recipes    |

Minimum prereqs before running `deploy.sh`:

- DNS `A` record `faucet.sophis.org → <VPS_IPv4>` (and `AAAA` for IPv6)
- Sudo-capable user (do not run as root)
- Ports 22 / 80 / 443 / 46211 (testnet P2P) reachable from the internet

### Co-located with a bootstrap node

The default deployment is co-located with one of the project's mainnet
bootstrap nodes — same VPS runs `sophisd-mainnet` (P2P 46111) for bootstrap
duty *and* `sophisd-testnet` + `testnet-faucet` + `nginx`. This keeps the
operational footprint to one VPS.

Recommended sizing: **2 vCPU / 4 GB RAM** (e.g. Hetzner CPX21, ~€8/mo). A
1 vCPU / 2 GB box can run a single node but gets tight with two `sophisd`
processes plus the faucet and nginx.

Trade-off: abuse on `/drip` can degrade the bootstrap node's responsiveness.
Mitigated by:

1. Cloudflare absorbing volumetric traffic at the edge (orange-cloud proxy)
2. nginx rate-limiting per IP before the upstream is hit
3. ufw blocking `:8181` (faucet upstream) and `:46110`/`:46210` (gRPC) from
   the public internet — only `:46111`/`:46211` (P2P) and `:80`/`:443` are
   public.

```bash
git clone https://github.com/sophis-network/Sophis /opt/sophis
cd /opt/sophis/testnet-faucet/deploy
DOMAIN=faucet.sophis.org ACME_EMAIL=ops@example.org ./deploy.sh
```

After `deploy.sh`:

1. Fund the faucet address (printed by the script) — send testnet SPHS to it
   from a separately-funded wallet, or temporarily point a miner at it.
2. Wait for `coinbase_maturity` (1000 blocks ≈ 100 min on testnet) if funding
   via mining, or instant if funding from an existing balance.
3. `curl https://faucet.sophis.org/status` — confirm `balance_sphs > amount_sphs`.

## CLI reference

```
testnet-faucet generate-wallet [--wallet PATH] [--network NETWORK]
testnet-faucet start
    --wallet PATH           (default: faucet_wallet.json)
    --rpcserver HOST:PORT   (default: localhost:46610)
    --network NETWORK       devnet | testnet | simnet | mainnet
    --port PORT             HTTP listen port (default: 8080)
    --amount SOMPI          drip size in sompi (default: 10⁹ = 10 SPHS)
    --cooldown SECONDS      per-address cooldown (default: 86400 = 24h)
    --history PATH          drip history JSON (default: faucet_history.json)
```

## API reference

### `GET /`

Returns the HTML faucet page. No auth.

### `GET /status`

```json
{
  "network": "testnet",
  "faucet_address": "sophistest:q...",
  "amount_sompi": 1000000000,
  "amount_sphs": 10.0,
  "cooldown_secs": 86400,
  "total_drips": 142,
  "total_sompi_sent": 142000000000,
  "total_sphs_sent": 1420.0,
  "balance_sompi": 50000000000,
  "balance_sphs": 500.0,
  "spendable_utxos": 23
}
```

### `POST /drip`

Request:
```json
{ "address": "sophistest:q..." }
```

Success — `200 OK`:
```json
{
  "tx_id": "<64-hex>",
  "amount_sompi": 1000000000,
  "amount_sphs": 10.0
}
```

Errors:

| Status | Body                                              | Meaning                                  |
|--------|---------------------------------------------------|------------------------------------------|
| 400    | `"Invalid address prefix..."`                     | Wrong network prefix                     |
| 400    | `"Invalid address: ..."`                          | Malformed bech32                         |
| 429    | `"This address already received SPHS..."`         | Cooldown active                          |
| 502    | `"Node rejected transaction: ..."`                | Node refused (storage mass, fee, etc.)   |
| 503    | `"Faucet has no spendable funds..."`              | Wallet balance < drip amount + fee       |
| 504    | `"RPC timeout..."`                                | Node unresponsive (15 s)                 |

## Operations playbook

### Daily health check

```bash
curl -fsS https://faucet.sophis.org/status | jq '.balance_sphs, .spendable_utxos, .total_drips'
```

If `balance_sphs` is approaching zero, top up the wallet. A simple alert:

```bash
# /etc/cron.d/faucet-balance
*/15 * * * * sophis curl -fsS http://localhost:8181/status | jq -e '.balance_sphs > 100' >/dev/null || \
    echo "Faucet balance low: $(curl -s http://localhost:8181/status | jq .balance_sphs)" | \
    mail -s '[faucet] low balance' ops@sophis.org
```

### Rotate the wallet

```bash
sudo systemctl stop faucet.service
sudo -u sophis testnet-faucet generate-wallet \
    --wallet /var/lib/sophis-faucet/wallet.new.json \
    --network testnet
# Move remaining balance from old → new with dilithium-wallet send
sudo mv /var/lib/sophis-faucet/wallet.json /var/lib/sophis-faucet/wallet.rotated-$(date +%F).json
sudo mv /var/lib/sophis-faucet/wallet.new.json /var/lib/sophis-faucet/wallet.json
sudo chown sophis:sophis /var/lib/sophis-faucet/wallet.json
sudo chmod 600 /var/lib/sophis-faucet/wallet.json
sudo systemctl start faucet.service
```

### Block an abusive IP

Two layers:

1. **Cloudflare** (preferred — see `deploy/cloudflare-rules.md`): WAF → Custom
   rules → block source IP for N hours.
2. **ufw** on the VPS: `sudo ufw insert 1 deny from <IP> to any`.

### Reset drip history

```bash
sudo systemctl stop faucet.service
sudo rm /var/lib/sophis-faucet/history.json
sudo systemctl start faucet.service
```

The history file is purely advisory — it implements the per-address cooldown.
Wiping it allows previously-served addresses to claim again.

## Abuse model

| Threat                            | Mitigation                                                |
|-----------------------------------|-----------------------------------------------------------|
| Same person, many addresses       | Cloudflare per-IP rate limit + Bot Fight                  |
| Same address, repeated claims     | `--cooldown` per-address (default 24h)                    |
| Datacenter farm IPs               | Cloudflare ASN block list (DigitalOcean, AWS, OVH, ...)   |
| Volumetric DDoS                   | Cloudflare absorbs at edge — origin only sees rate-limited|
| Wallet key theft (host compromise)| Wallet holds ≤ a few weeks of drip volume; rotate often   |
| Node compromise → exfiltrate keys | Wallet on faucet host, not on node; separate services     |

Out of scope (testnet tokens have no monetary value, by design):

- Sybil resistance beyond what Cloudflare provides
- Captcha / proof of personhood
- KYC

## Common errors and what they mean

### `502 Node rejected transaction: ... storage mass of X is larger than max`

The drip amount is too small relative to the change output. Consensus storage
mass formula penalizes small outputs disproportionately. Two fixes:

- **Increase `--amount`**. For testnet with ~10 SPHS coinbase rewards, drips
  of ≥ 1 SPHS keep storage mass well under the 10⁷ ceiling.
- **Consolidate the wallet's UTXOs** with a separate spend before continuing.

In devnet (low per-block reward → many tiny UTXOs), drips below ~0.01 SPHS
will frequently trip this on a fresh chain.

### `503 Faucet has no spendable funds`

The wallet has either:
- No UTXOs yet (waiting for first mined block)
- All UTXOs still immature (need `coinbase_maturity` confirmations)
- Balance < `amount + fee`

`curl /status` to inspect.

### `504 RPC timeout submitting transaction`

The local `sophisd` is unresponsive. Check `systemctl status sophisd-testnet`
and tail its log. RocksDB compaction or peer sync can briefly block the gRPC
thread; transient timeouts are normal under load.

### Faucet keeps restarting

```bash
sudo journalctl -u faucet -n 100
```

Most common cause: `--wallet` file missing or unreadable. Second most common:
sophisd not reachable on `--rpcserver`. Third: `--amount` greater than what
the wallet can ever spend (no UTXO is big enough on its own and consolidation
fails repeatedly).

## License

AGPL-3.0 — see repository root.
