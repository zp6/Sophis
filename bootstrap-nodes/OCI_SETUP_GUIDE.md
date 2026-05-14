# Sophis Bootstrap Nodes — Oracle Cloud Always Free guide

Step-by-step to provision **2 bootstrap nodes** on Oracle Cloud Infrastructure
(OCI) **Always Free tier**, in two geographic regions, at zero recurring cost.

> **Free tier limits used here**: 2× `VM.Standard.A1.Flex` (ARM Ampere) with
> 2 OCPUs + 12 GB RAM each (total 4 OCPU + 24 GB, exactly the Always Free cap).
> 100 GB block storage total. 10 TB egress/month — far more than any bootstrap
> node will ever consume.

## What you'll end up with

```
┌─────────────────────────────────────┐    ┌─────────────────────────────────────┐
│ bootstrap1.sophis.org  (São Paulo)  │    │ bootstrap2.sophis.org  (Ashburn)    │
│                                     │    │                                     │
│  sophisd-mainnet  :46111  ─── P2P ──┼────┼─── :46111  sophisd-mainnet          │
│  sophisd-testnet  :46211  ─── P2P ──┼────┼─── :46211  sophisd-testnet          │
│  testnet-faucet   :8181   (internal)│    │                                     │
│  nginx + certbot  :80/443           │    │                                     │
│  faucet.sophis.org                  │    │                                     │
└─────────────────────────────────────┘    └─────────────────────────────────────┘
   Public IPv4 (Reserved)                    Public IPv4 (Reserved)
```

## Prerequisites checklist

Before you start, have these ready:

- [ ] Oracle Cloud account (Always Free signup at https://www.oracle.com/cloud/free/)
- [ ] Credit card for signup verification (won't be charged on Always Free)
- [ ] A domain name with DNS managed by Cloudflare (or any DNS host) —
      `sophis.org` already meets this
- [ ] SSH key pair on your workstation (`ssh-keygen -t ed25519 -C "sophis-ops"`
      generates `~/.ssh/id_ed25519` + `id_ed25519.pub` if you don't have one)
- [ ] About 90 minutes total: 15 min provisioning, 45 min build, 30 min wiring

---

## Phase 1 — Subscribe to a second region

OCI accounts default to one "home region." For a 2-geo deploy, subscribe to
a second region first (this takes ~10 minutes to activate).

1. Sign in to https://cloud.oracle.com
2. Top-right corner → click the **region dropdown** (probably says "US East
   (Ashburn)" or your signup region) → **Manage Regions**
3. Find **São Paulo (sa-saopaulo-1)** in the list → click **Subscribe**
4. Wait ~10 min for activation (you'll get an email)

**Recommended regions for Sophis:**

| Home region                | Second region              | Why                                                      |
|----------------------------|----------------------------|----------------------------------------------------------|
| São Paulo (sa-saopaulo-1)  | Ashburn (us-ashburn-1)     | BR latency + US-East hub. Default for this guide.        |
| São Paulo                  | Frankfurt (eu-frankfurt-1) | BR + EU jurisdictional diversity                         |
| Ashburn                    | Frankfurt                  | No BR presence but lowest global latency for dev hubs    |

If you started in the wrong home region, set your *most-used* region as home
and use the other as secondary — both behave identically.

---

## Phase 2 — Provision bootstrap1 (São Paulo)

### 2.1 Create the VCN (Virtual Cloud Network)

1. Switch region to **São Paulo** (top-right dropdown)
2. Hamburger menu (top-left) → **Networking → Virtual Cloud Networks**
3. **Create VCN** (use the wizard "Create VCN with Internet Connectivity")
   - Name: `sophis-vcn-sp`
   - VCN CIDR: `10.0.0.0/16` (default fine)
   - Public Subnet CIDR: `10.0.0.0/24` (default fine)
   - Click **Create** → wait ~30s

The wizard creates: VCN, public subnet, internet gateway, route table, default
security list (with SSH already open). We'll edit the security list in step 2.4.

### 2.2 Reserve a public IP (so we can DNS-target it)

1. **Networking → Reserved Public IPs** → **Reserve Public IP Address**
   - Name: `sophis-bootstrap-1-ip`
   - Compartment: root (your tenancy name)
   - Click **Reserve Public IP Address**
2. Copy the IP shown — this is your bootstrap1 IP. Save it.

### 2.3 Create the instance

1. **Compute → Instances → Create Instance**
2. Fill in:
   - **Name**: `sophis-bootstrap-1`
   - **Compartment**: root
   - **Placement**: any AD (Availability Domain) — pick AD-1 if asked
   - **Image and shape**:
     - Image: **Canonical Ubuntu 24.04** (click "Change image" if not default)
     - Shape: click **Change shape** → switch to **Ampere** tab → select
       `VM.Standard.A1.Flex` → set **OCPUs: 2**, **Memory (GB): 12**
   - **Primary VNIC information**:
     - Virtual cloud network: `sophis-vcn-sp` (just created)
     - Subnet: the public subnet inside it
     - **Assign a public IPv4 address**: yes
     - **Use existing reserved IP**: select `sophis-bootstrap-1-ip`
   - **SSH keys**: paste your public key (contents of
     `~/.ssh/id_ed25519.pub`) — this is the ONLY way you'll reach the box
   - **Boot volume**: set size to **50 GB** (default 47 GB also works)
3. **Advanced options** (collapse expand at bottom):
   - **Management → Initialization script** → "Paste cloud-init script"
   - Paste the entire contents of
     `bootstrap-nodes/cloud-init/bootstrap1-cloud-init.yaml` (file in this
     repo)
4. Click **Create**. Provisioning takes ~60 seconds.

### 2.4 Open the P2P ports in the Security List

Cloud-init opens iptables inside the VM, but OCI's edge security list also
blocks inbound traffic by default. Open the P2P ports:

1. **Networking → Virtual Cloud Networks → sophis-vcn-sp → Security Lists →
   Default Security List**
2. **Add Ingress Rules** (the button) — add these one at a time:

| Source CIDR  | IP Protocol | Source Port | Destination Port | Description         |
|--------------|-------------|-------------|------------------|---------------------|
| `0.0.0.0/0`  | TCP         | (blank)     | `46111`          | Sophis mainnet P2P  |
| `0.0.0.0/0`  | UDP         | (blank)     | `46111`          | Sophis mainnet P2P  |
| `0.0.0.0/0`  | TCP         | (blank)     | `46211`          | Sophis testnet P2P  |
| `0.0.0.0/0`  | UDP         | (blank)     | `46211`          | Sophis testnet P2P  |
| `0.0.0.0/0`  | TCP         | (blank)     | `80`             | HTTP ACME challenge |
| `0.0.0.0/0`  | TCP         | (blank)     | `443`            | HTTPS faucet        |

**Do not** open `46110`/`46210` (gRPC), `47110`/`47210` (Borsh), `48110`/`48210`
(JSON), or `8181` (faucet upstream). Those stay localhost-only.

### 2.5 Wait for cloud-init to finish

```bash
ssh ubuntu@<bootstrap-1-ip>
# Watch the build progress (≈ 30–45 min on Ampere A1 2-OCPU)
sudo tail -f /var/log/cloud-init-output.log
# When you see "Sophis bootstrap node ready", cloud-init is done.
```

If you SSH in immediately after instance creation, you'll see the build
running. Don't interrupt it. The instance is fully usable when cloud-init
finishes and `systemctl status sophisd-mainnet` shows `active (running)`.

---

## Phase 3 — Provision bootstrap2 (Ashburn)

Repeat Phase 2 with these differences:

1. **Switch region** to Ashburn (top-right dropdown)
2. **Name everything** with `-2` suffix: `sophis-vcn-va`, `sophis-bootstrap-2-ip`,
   `sophis-bootstrap-2`
3. **Cloud-init**: paste `bootstrap-nodes/cloud-init/bootstrap2-cloud-init.yaml`
   (no faucet/nginx — bootstrap2 is node-only)
4. **Security list**: open only `46111` and `46211` (TCP+UDP) — no `80`/`443`
5. **After both are up**, edit `bootstrap2`'s `/etc/systemd/system/sophisd-mainnet.service`
   to add `--connect=<bootstrap-1-public-ip>:46111`. This isn't strictly
   needed (peer discovery works) but guarantees the two find each other
   immediately:

   ```bash
   ssh ubuntu@<bootstrap-2-ip>
   sudo sed -i "s|--utxoindex|--utxoindex \\\\\n    --connect=<bootstrap-1-ip>:46111|" \
       /etc/systemd/system/sophisd-mainnet.service
   sudo systemctl daemon-reload
   sudo systemctl restart sophisd-mainnet
   ```

---

## Phase 4 — DNS records (Cloudflare)

In Cloudflare → `sophis.org` zone → **DNS → Records → Add record** (one at a
time):

| Type | Name         | Content              | Proxy        | TTL  |
|------|--------------|----------------------|--------------|------|
| `A`  | `bootstrap1` | `<bootstrap-1-ip>`   | **OFF (DNS only)** | Auto |
| `A`  | `bootstrap2` | `<bootstrap-2-ip>`   | **OFF (DNS only)** | Auto |
| `A`  | `faucet`     | `<bootstrap-1-ip>`   | ON (proxied) | Auto |

> **Why proxy OFF for bootstrap1/2**: Cloudflare's HTTP proxy can't relay
> arbitrary TCP/UDP — P2P traffic needs a real IP. Faucet uses HTTPS so it
> can (and should) sit behind the proxy for DDoS protection.

Verify:
```bash
dig +short bootstrap1.sophis.org   # → <bootstrap-1-ip> (real IP)
dig +short bootstrap2.sophis.org   # → <bootstrap-2-ip> (real IP)
dig +short faucet.sophis.org       # → 104.x.x.x or similar (Cloudflare)
```

---

## Phase 5 — Verify the cluster

After both VMs are running and DNS has propagated (~5 min):

```bash
# 1. Both nodes alive
ssh ubuntu@bootstrap1.sophis.org "systemctl is-active sophisd-mainnet sophisd-testnet"
ssh ubuntu@bootstrap2.sophis.org "systemctl is-active sophisd-mainnet sophisd-testnet"
# Expected: all four lines say "active"

# 2. They see each other
ssh ubuntu@bootstrap1.sophis.org \
    "sudo -u sophis sophis-cli get-connected-peer-info 2>/dev/null | grep -c address"
# Expected: ≥ 1 (other bootstrap + any organic peers)

# 3. Faucet reachable (after Phase 6 of the faucet deploy.sh, which runs
#    automatically via cloud-init)
curl -fsS https://faucet.sophis.org/status | jq .network
# Expected: "testnet"
```

If any of these fail, see `bootstrap-nodes/BOOTSTRAP_RUNBOOK.md`
("Troubleshooting") for diagnosis steps.

---

## Phase 6 — Monitoring (free tier)

Set up uptime monitoring with **UptimeRobot** (free 50 monitors, 5-min interval):

1. Sign up at https://uptimerobot.com (free, no card)
2. Add monitors:

| Type          | Name                       | URL/Host                                    |
|---------------|----------------------------|---------------------------------------------|
| HTTP(s)       | `faucet`                   | `https://faucet.sophis.org/status`          |
| Port (TCP)    | `bootstrap1 P2P mainnet`   | `bootstrap1.sophis.org:46111`               |
| Port (TCP)    | `bootstrap2 P2P mainnet`   | `bootstrap2.sophis.org:46111`               |
| Port (TCP)    | `bootstrap1 P2P testnet`   | `bootstrap1.sophis.org:46211`               |
| Port (TCP)    | `bootstrap2 P2P testnet`   | `bootstrap2.sophis.org:46211`               |

3. Alerts → add your email (free) or Telegram (free). Set "Down for 5 minutes"
   threshold to avoid flapping.

That's it. Cost: $0/mo. Free tier supports this exact setup forever.

---

## Common pitfalls

- **"Out of capacity" error when creating the A1.Flex instance**: Ampere
  capacity is sometimes constrained per region. Try a different Availability
  Domain (AD-1 → AD-2 → AD-3 dropdown during creation), or retry in a few
  hours. The free tier instances ARE available; capacity rebalancing happens
  hourly.

- **SSH connection refused**: The OCI Security List blocks SSH by default
  from anywhere — *no it doesn't*, the default rule allows TCP/22 from
  0.0.0.0/0. If you can't connect, the VM is still booting or your
  `ufw`/`iptables` got over-strict during cloud-init. Use the OCI Console's
  **Cloud Shell** as a fallback to reach the VM.

- **"This Always Free instance was reclaimed"**: Auto-reclaim only triggers
  if CPU < 20%, network < 50 MB, AND memory < 50% for 7 days continuously.
  A running `sophisd` never satisfies this. You're safe.

- **Slow build (>1h)**: A1 sometimes throttles on heavy compilation. Set
  `CARGO_BUILD_JOBS=2` in `.bashrc` of the sophis user. Already done by
  cloud-init.

- **`certbot` fails on first run**: Means DNS hasn't propagated yet or port
  80 isn't reachable. Confirm `dig +short faucet.sophis.org` returns an IP,
  then re-run `sudo certbot --nginx -d faucet.sophis.org --non-interactive
  --agree-tos -m ops@example.org` manually.

- **iptables rules disappear after reboot**: `netfilter-persistent save`
  must run after adding rules. Cloud-init does this. If you add rules
  manually later, run it again.

---

## What this deployment does NOT include

The following are deliberately out of scope for the bootstrap nodes (see
`MEMORY.md` index for context):

- **DNS seeder** (item #2 of the roadmap) — separate component, runs on
  bootstrap1 or a third host
- **Indexer Phase 9** (item #11) — runs on a separate VPS; gRPC clients to
  bootstrap1
- **Dashboard production deploy** (item #10) — static site or separate VPS
- **Phase 6 DA stress test** (item #13) — uses bootstraps as targets but
  runs from a separate test harness

Each of those gets its own setup once these bootstraps are healthy.
