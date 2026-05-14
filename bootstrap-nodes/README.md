# Sophis bootstrap nodes — OCI Always Free deployment

Production deployment of 2 bootstrap nodes on **Oracle Cloud Infrastructure
(OCI) Always Free tier** — total cost $0/mo, two geographic regions.

## Files

| File                                       | Purpose                                                |
|--------------------------------------------|--------------------------------------------------------|
| [`OCI_SETUP_GUIDE.md`](OCI_SETUP_GUIDE.md) | OCI Always Free provisioning ($0/mo, 12 GB RAM)        |
| [`HETZNER_SETUP_GUIDE.md`](HETZNER_SETUP_GUIDE.md) | Hetzner CX22 provisioning (~€8.60/mo, 4 GB RAM) |
| [`BOOTSTRAP_RUNBOOK.md`](BOOTSTRAP_RUNBOOK.md) | Day-to-day operations after provisioning            |
| `cloud-init/bootstrap1-cloud-init.yaml`    | Cloud-init for bootstrap1 (with faucet co-located)     |
| `cloud-init/bootstrap2-cloud-init.yaml`    | Cloud-init for bootstrap2 (node-only)                  |

## What this is

- **2 VMs**, one per region (São Paulo + Ashburn by default — pickable)
- **`VM.Standard.A1.Flex` ARM Ampere** — 2 OCPU + 12 GB RAM each
- **Always Free** — never expires, never charges
- Builds `sophisd` from source on first boot (≈ 45 min)
- Runs both **mainnet** P2P (`:46111`) and **testnet** P2P (`:46211`)
- Bootstrap1 additionally hosts the **testnet faucet** at `https://faucet.sophis.org`
- Reserved public IPs so DNS records stay stable across reboots

## Why OCI

| Provider              | Cost for this config                       | Notes                                          |
|-----------------------|--------------------------------------------|------------------------------------------------|
| **OCI Always Free**   | $0/mo forever                              | 2× A1.Flex (ARM), 200 GB storage, 10 TB egress |
| Hetzner CPX21         | ~€8/mo × 2 = ~R$96/mo                      | The previous recommendation (Task F)           |
| DigitalOcean Basic    | $6/mo × 2 = ~R$72/mo                       |                                                |
| AWS t4g.small         | ~$15/mo × 2 = ~R$180/mo (after free tier)  | + KYC pain                                     |

OCI wins on cost. Trade-off: vendor lock-in to one platform — if Oracle changes
the Always Free terms, migrate to one of the paid options above. The cloud-init
YAML is mostly portable; only the OCI-specific iptables persistence and
Security List config would need adjusting.

## What this is NOT

- **Not a managed service** — you SSH in, you debug, you patch
- **Not high-availability** — 2 nodes, no auto-failover, ~99.5% effective uptime
- **Not the whole network** — just bootstrap. Real validators/miners run on
  their own hardware

## Operational expectations

- **First 30 days**: check daily, build muscle memory on the runbook
- **After 30 days**: weekly check is fine; UptimeRobot alerts on outages
- **Updates**: rebuild + restart when consensus-impacting changes land on
  `main` (~monthly). Non-consensus changes don't require bootstrap rebuilds
- **Wallet refill** (faucet): top up every 2–4 weeks based on drip volume

## Related items in the roadmap

This deployment closes **item #1** of the user-must-do roadmap and unlocks:

- **#2 DNS seeders** — runs as a third process on bootstrap1 or on a separate host
- **#3 Faucet hosting** — already included here (co-located on bootstrap1)
- **#10 I1 Dashboard** — consumes gRPC from one of the bootstraps
- **#11 Phase 9 indexer** — same: connects to bootstrap1 gRPC over a private path
- **#13 Phase 6 DA stress test** — uses both bootstraps as targets

See `MEMORY.md` index entry "ROADMAP USER-MUST-DO" for the full sequencing.
