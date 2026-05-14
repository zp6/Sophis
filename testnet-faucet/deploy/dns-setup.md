# DNS setup — `faucet.sophis.org`

`sophis.org` is hosted at Cloudflare (Cloudflare Pages for the static site).
The faucet subdomain is added in the same zone.

## Step 1 — pick the target

Pick which of these you're doing first, then create the matching record:

| Target                         | Record type | Content        | Notes                                           |
|--------------------------------|-------------|----------------|-------------------------------------------------|
| **VPS exists, deploy ready**   | `A`         | `<VPS_IPv4>`   | Proxy: ON (orange cloud)                        |
| **VPS exists, IPv6 too**       | `AAAA`      | `<VPS_IPv6>`   | Proxy: ON                                       |
| **No VPS yet — reserve name**  | `TXT`       | `"reserved"`   | Placeholder; replace with A/AAAA when deploying |

> If you go with the TXT placeholder, the subdomain resolves but doesn't serve
> anything. You can swap it for `A`/`AAAA` later in seconds — no propagation
> wait since Cloudflare TTL is Auto (≈ 5 min).

## Step 2 — create the record

In the Cloudflare dashboard:

1. Select the `sophis.org` zone.
2. **DNS → Records → Add record**
3. Fill in:
   - **Type**: `A` (or `AAAA` or `TXT` per the table above)
   - **Name**: `faucet`
   - **Content**: per the table above
   - **Proxy status**: ON (for `A`/`AAAA`), N/A (for `TXT`)
   - **TTL**: Auto
4. Save.

## Step 3 — verify

```bash
dig +short faucet.sophis.org           # should return Cloudflare's edge IPs (104.x / 172.x) when proxied
dig +short faucet.sophis.org @1.1.1.1  # same — independent resolver

# If TXT placeholder:
dig +short TXT faucet.sophis.org
```

## Step 4 — when ready to deploy

Once the VPS is provisioned and `deploy.sh` is ready:

1. Edit the `faucet` DNS record → change Type from `TXT` to `A`.
2. Set Content to the VPS IPv4. Proxy ON. Save.
3. Wait ≈ 2 min for the change to propagate.
4. Run `deploy.sh` on the VPS — `certbot` will issue a real cert via HTTP-01
   challenge (Cloudflare proxy is configured to pass `/.well-known/` through).

## Step 5 — Cloudflare WAF rules

After the A record is live and `deploy.sh` succeeded, apply the WAF and bot
mitigation rules from [`cloudflare-rules.md`](cloudflare-rules.md).

---

## What this gives you

- **`faucet.sophis.org` resolves immediately** after step 2 — you can wire
  third parties or docs to the URL before the service exists.
- **Free migration**: if a volunteer takes over hosting later, you only edit
  the DNS record. No code, no domain change, no user-visible URL change.
- **Operational isolation**: abuse, downtime, or rotation on the faucet does
  not affect `sophis.org` itself.
