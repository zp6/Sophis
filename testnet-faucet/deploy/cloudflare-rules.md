# Cloudflare rules for faucet.sophis.org

If `faucet.sophis.org` is proxied through Cloudflare (orange-cloud), add these
in addition to the nginx-level rate limiting. Free plan covers all of this.

## DNS

| Type | Name   | Content        | Proxy | TTL  |
|------|--------|----------------|-------|------|
| A    | faucet | `<VPS_IPv4>`   | ON    | Auto |
| AAAA | faucet | `<VPS_IPv6>`   | ON    | Auto |

## Rate-limiting rules

Cloudflare → Security → WAF → Rate limiting rules.

### Rule 1: `/drip` — 3 requests per minute per IP

- **Expression**: `(http.request.uri.path eq "/drip" and http.request.method eq "POST")`
- **Characteristics**: IP address
- **Requests**: 3
- **Period**: 1 minute
- **Action**: Block
- **Duration**: 10 minutes

### Rule 2: global — 30 requests per minute per IP

- **Expression**: `(http.host eq "faucet.sophis.org")`
- **Characteristics**: IP address
- **Requests**: 30
- **Period**: 1 minute
- **Action**: Managed Challenge
- **Duration**: 5 minutes

## Bot Fight Mode

Security → Bots → Bot Fight Mode → **ON** (free plan).

Stops scripted abuse from datacenter IPs (most common faucet attacker source).

## Custom firewall rules

Security → WAF → Custom rules.

### Block known abusive ASNs (optional, tune to your traffic)

- **Expression**: `(ip.geoip.asnum in {14061 16509 16276 24940})`  
  *(DigitalOcean, AWS, OVH, Hetzner — datacenter ranges)*
- **Action**: Managed Challenge

Be cautious: blocking AWS also blocks legitimate devs using AWS-hosted dev
boxes. Start with Challenge, escalate to Block only if abuse persists.

### Country gate (only if you must)

- **Expression**: `(ip.geoip.country in {"CN" "RU" "KP" "IR"})`
- **Action**: Managed Challenge

Note: a faucet is for testnet tokens with no monetary value. Blocking
countries is rarely worth the false-positive cost.

## Caching

- **Cache Rules**: do NOT cache `/drip` (POST). The `/` and `/status` endpoints
  can be cached for ~30s if traffic justifies — usually not necessary.

## Page Rules / Transform Rules

None required. Default settings work.

## Logs

- Cloudflare → Analytics → Security → events visible for 24h (free plan), 30d
  on paid plans.
- Cross-reference with `/var/log/nginx/faucet.access.log` for full request
  picture.
