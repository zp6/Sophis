# Hetzner Cloud CX22 — Sophis bootstrap nodes setup

End-to-end provisioning of the two Sophis bootstrap nodes on **Hetzner
Cloud** CX22 instances (2 vCPU / 4 GB RAM / 40 GB NVMe) across two
regions. Cost: ~€8.60/month total (≈ R$50).

Alternative to [`OCI_SETUP_GUIDE.md`](OCI_SETUP_GUIDE.md). Pick one path.
The cloud-init YAMLs in `cloud-init/` work unchanged on both providers
(the swap step they ship with is no-op on the larger OCI A1.Flex).

## Provider comparison

| Provider              | Monthly cost | RAM   | Trade-off                                  |
|-----------------------|--------------|-------|--------------------------------------------|
| **OCI Always Free**   | $0           | 12 GB | Vendor lock-in; auto-reclaim risk          |
| **Hetzner CX22 × 2**  | ~€8.60       | 4 GB  | Paid, but no surprise suspension           |
| Hetzner CPX21 × 2     | ~€16         | 8 GB  | Comfort margin if CX22 OOMs persistently   |
| DigitalOcean Basic ×2 | ~$12         | 1 GB  | Too small for native build                 |

Hetzner CX22 fits the build only with the swap + `CARGO_BUILD_JOBS=1`
adjustments baked into the cloud-init. Without them, the `cargo build
--release` step OOMs at the linker stage every time.

## Prerequisites

- Cartão de crédito internacional (Visa/Mastercard) — debit/prepaid usually rejected
- An email you control (Hetzner sends sign-up verification)
- A workstation with `ssh` available (Windows 11: PowerShell; macOS/Linux: any terminal)
- Cloudflare account managing `sophis.org` DNS (used in §9)
- ~2 hours wall-clock (15 min hands-on + 90 min unattended build × 2)

## 1. Create the Hetzner Cloud account

1. Go to <https://hetzner.com/cloud> → **Sign up**.
2. Fill in name, address, document number (in BR: CPF works in the
   "tax ID / VAT" field — Hetzner accepts non-EU IDs).
3. Verify the email link they send.
4. Open <https://console.hetzner.cloud> and log in.
5. Settings → **Billing** → add a credit card. Hetzner places a small
   pre-auth charge (€0–€1) that drops within 48 h.
6. Account review may take **1 h to 24 h**. New accounts sometimes get
   a manual KYC review — if they email asking for ID, send a passport
   or driver's license photo. Don't skip this; without verification you
   can't create servers.

> ⚠️ **Account suspension risk:** Hetzner is stricter on new accounts
> than older ones. Don't try unusual operations in the first 7 days
> (no Tor, no port scanning, no high-volume egress). Just provision,
> SSH in, run sophisd. After ~30 days the account is "seasoned" and
> the anti-fraud bar drops considerably.

## 2. Generate an SSH key (skip if you already have one)

**Windows 11 PowerShell:**

```powershell
ssh-keygen -t ed25519 -C "sophis-bootstrap" -f $HOME\.ssh\sophis_bootstrap
```

Press Enter twice (no passphrase — Windows DPAPI protects the file).

**macOS / Linux:**

```bash
ssh-keygen -t ed25519 -C "sophis-bootstrap" -f ~/.ssh/sophis_bootstrap
```

The public key ends in `.pub`. Print it to copy:

```powershell
# Windows
type $HOME\.ssh\sophis_bootstrap.pub
```

```bash
# macOS/Linux
cat ~/.ssh/sophis_bootstrap.pub
```

Copy the full single-line output. You'll paste it into Hetzner in §3.

## 3. Create the Sophis project

1. Hetzner Cloud Console → top-right **+** → **New Project**
2. Name: `sophis-bootstrap` → Create
3. Click into the project → **Security** tab → **SSH Keys** → **Add
   SSH Key** → paste the public key from §2 → name it `workstation` →
   **Add SSH Key**

The key is now reusable across all servers in this project.

## 4. Provision bootstrap1 (with faucet)

1. **Servers** tab → **Add Server**
2. **Location**: `Ashburn, VA` (US East — lowest latency to Brazil)
3. **Image**: `Ubuntu 24.04`
4. **Type** → Shared vCPU tab → **CX22** (€3.79/mo + €0.50 IPv4)
5. **Networking**: leave Public IPv4 checked
6. **SSH Keys**: select `workstation`
7. **Cloud config**: paste the **entire contents** of
   [`cloud-init/bootstrap1-cloud-init.yaml`](cloud-init/bootstrap1-cloud-init.yaml)
   (open the file in your text editor, Ctrl-A, Ctrl-C, paste).
   **Do NOT edit the YAML** — it already ships with the CX22-sized
   swap file and `CARGO_BUILD_JOBS=1` baked in. The `if !
   swapon...` block early in `runcmd` is the swap step; touching it
   risks YAML indentation errors that abort cloud-init silently.
8. **Name**: `sophis-bootstrap1`
9. **Create & Buy now**

Hetzner shows the new server with an IPv4 like `5.161.x.x`. Note it
down — you'll SSH in to verify, and you'll set Cloudflare DNS to it
in §9.

## 5. Provision bootstrap2 (node-only)

Repeat §4 with these differences:

- **Location**: `Falkenstein, DE` (Germany — different continent so a
  regional outage doesn't take both down)
- **Cloud config**: paste [`cloud-init/bootstrap2-cloud-init.yaml`](cloud-init/bootstrap2-cloud-init.yaml)
- **Name**: `sophis-bootstrap2`

> 💡 **Location matrix:** Hetzner offers Falkenstein/Nuremberg/Helsinki
> (Europe) and Ashburn/Hillsboro (US) and Singapore. Pick two from
> different continents. Avoid Singapore for Brazil-facing latency
> reasons. Falkenstein + Ashburn is the default recommendation.

## 6. Wait for cloud-init to finish (45–90 min)

Both servers are reachable via SSH immediately, but the build is still
running in the background. To follow along:

```powershell
# Windows
ssh -i $HOME\.ssh\sophis_bootstrap root@<bootstrap1-ip> "tail -f /var/log/cloud-init-output.log"
```

```bash
# macOS/Linux
ssh -i ~/.ssh/sophis_bootstrap root@<bootstrap1-ip> "tail -f /var/log/cloud-init-output.log"
```

> ⚠️ **Hetzner default user is `root`, not `ubuntu`.** The
> [`BOOTSTRAP_RUNBOOK.md`](BOOTSTRAP_RUNBOOK.md) was written for OCI
> and uses `ubuntu@`. On Hetzner, replace `ubuntu@` with `root@` in
> every operational command, or create a non-root user — see §10.

Expected progression:

- Minute 0–2: package install (`apt update && apt upgrade`)
- Minute 2–8: Rust toolchain install (rustup)
- Minute 8–15: git clone Sophis + cargo registry download
- Minute 15–75: `cargo build --release` (the long one — coffee break)
- Minute 75–85: install binaries, enable systemd units, faucet wallet
  generation
- Minute 85–90: `Sophis bootstrap node #1 (with faucet) ready.`

If the build OOMs anyway (rare with swap + `BUILD_JOBS=1` but possible):

```bash
# SSH in and re-run manually with even tighter limits
ssh root@<ip>
sudo -u sophis bash -lc '
  export LIBCLANG_PATH=$(llvm-config --libdir)
  export CARGO_BUILD_JOBS=1
  export RUSTFLAGS="-C codegen-units=1"
  cd /var/lib/sophis/src
  ~/.cargo/bin/cargo build --release -p sophisd -p testnet-faucet -p dilithium-wallet
'
```

Adding `codegen-units=1` shrinks peak linker memory at the cost of
~20 extra minutes.

## 7. Verify both nodes are healthy

```powershell
# Windows: run for both servers
ssh -i $HOME\.ssh\sophis_bootstrap root@<bootstrap1-ip> `
    "systemctl is-active sophisd-mainnet sophisd-testnet faucet"

ssh -i $HOME\.ssh\sophis_bootstrap root@<bootstrap2-ip> `
    "systemctl is-active sophisd-mainnet sophisd-testnet"
```

Expected (bootstrap1): `active`, `active`, `active`
Expected (bootstrap2): `active`, `active`

If anything says `failed` or `inactive`, jump to §11 Troubleshooting.

## 8. (bootstrap1 only) Set the faucet HTTPS certificate

The cloud-init left nginx running on port 80 only. To enable
`https://faucet.sophis.org`, first point DNS (§9) then SSH in:

```bash
ssh root@<bootstrap1-ip>
sudo certbot --nginx -d faucet.sophis.org --non-interactive \
    --agree-tos --email <your-email> --redirect --hsts
```

Certbot edits the nginx vhost in place, reloads it, and sets up a
renewal cron. Verify:

```bash
curl -sS https://faucet.sophis.org/status | jq '.balance_sphs'
```

Should print `0` (faucet wallet exists, balance not funded yet — see §10).

## 9. Cloudflare DNS

In your Cloudflare dashboard for `sophis.org`:

| Type | Name         | Content                | Proxy status |
|------|--------------|------------------------|--------------|
| A    | `bootstrap1` | `<bootstrap1-ip>`      | **DNS only** (gray cloud) |
| A    | `bootstrap2` | `<bootstrap2-ip>`      | **DNS only** (gray cloud) |
| A    | `faucet`     | `<bootstrap1-ip>`      | **Proxied** (orange cloud) |

> ⚠️ **P2P needs proxy OFF.** Cloudflare's proxy only forwards
> HTTP/HTTPS; the Sophis P2P protocol on port 46111/46211 won't work
> through it. Bootstrap A-records must be `DNS only`.

Wait 60 s for DNS propagation, then test:

```bash
nslookup bootstrap1.sophis.org    # should return the Hetzner IP
curl -sI https://faucet.sophis.org/ | head -1   # should return 200 OK
```

## 10. Fund the faucet wallet

```bash
ssh root@<bootstrap1-ip>
sudo jq -r .address /var/lib/sophis-faucet/wallet.json
```

That prints a `sophistest:q...` address. Send testnet SPHS to it from
a wallet that has them (mine some yourself, or ask in the testnet
chat). Faucet picks the funds up after `coinbase_maturity` (1000
blocks on testnet ≈ 100 min).

## 11. Troubleshooting

### Build still OOMs

```bash
ssh root@<ip>
free -h          # confirm swap is on; should show "Swap: 8.0Gi"
sudo systemctl status sophisd-mainnet
sudo journalctl -u sophisd-mainnet -n 100 --no-pager
```

If swap shows `0Gi`, the swap step in cloud-init failed. Recover:

```bash
ssh root@<ip>
fallocate -l 8G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile
echo '/swapfile none swap sw 0 0' >> /etc/fstab
# Then re-run the build manually (see §6)
```

### "Connection refused" on SSH

- Wait 60 s after server provisioning — cloud-init takes a moment to
  enable the SSH service
- Verify the public key in the Hetzner project matches what's in your
  workstation's `~/.ssh/sophis_bootstrap.pub`
- Try the **Console** button in Hetzner — it opens a browser-based
  terminal that bypasses SSH and lets you debug from inside

### Build finishes but `sophisd-mainnet` is `failed`

```bash
ssh root@<ip>
sudo journalctl -u sophisd-mainnet -n 50 --no-pager
```

Common cause: the binary install step skipped because the build
target dir doesn't exist (build failed silently). Re-run the build
step from §6.

### Hetzner suspended my account after 1 day

This happens to new accounts sometimes. Email
`support@hetzner.com` from your account email with:

- "Hi, account just suspended; using it for a personal open-source
  blockchain project (sophis.org). Happy to provide ID if helpful."
- Attach a passport or driver's license photo

Response time: 4–24 h. Don't try to recreate the account from another
email — that flags both as fraud.

### Performance complaints / "node is slow"

CX22 is the entry tier. After mainnet launch, if peer count drops or
blocks lag, upgrade in place:

1. Console → server → **Rescale**
2. Pick **CPX21** (€7.55/mo, 3 vCPU shared, 4 GB RAM, 80 GB disk) or
   **CPX31** (€14.05/mo, 4 vCPU dedicated, 8 GB RAM, 160 GB disk)
3. Server reboots (~30 s); systemd auto-starts sophisd

## 12. Migration path off Hetzner

If you ever need to move (price hike, account issue, geo change):

1. Provision the target VM elsewhere (DigitalOcean, OVH, Vultr — all
   accept the same cloud-init format)
2. Wait for the new node to be in sync with peers (~minutes pre-mainnet,
   ~hours post-mainnet at chain head)
3. Update the Cloudflare A record to the new IP
4. Wait 5 min for DNS TTL
5. Terminate the Hetzner server

The chain re-syncs from peers; nothing on the bootstrap is irreplaceable
**except the faucet wallet** at `/var/lib/sophis-faucet/wallet.json`.
Back that up before terminating (see `BOOTSTRAP_RUNBOOK.md` → Backups).

## 13. Cost monitoring

Hetzner bills monthly. Expected items:

| Item                          | Cost          |
|-------------------------------|---------------|
| CX22 server × 2               | €7.58/mo      |
| Public IPv4 × 2               | €1.00/mo      |
| Traffic (under 20 TB/server)  | €0.00/mo      |
| **Total**                     | **~€8.58/mo** |

Check Console → **Billing** at month-end. Anything over €10 deserves
investigation (Volume you forgot to delete; Floating IP; etc.).
