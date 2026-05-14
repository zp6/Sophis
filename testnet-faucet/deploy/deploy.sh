#!/usr/bin/env bash
# deploy.sh — Idempotent deploy script for Sophis testnet faucet on Ubuntu 22.04/24.04 LTS
#
# Run as a user with sudo. Adjust the variables at the top, then:
#   ./deploy.sh
#
# What it does (idempotent — safe to re-run):
#   1. Installs nginx, certbot, ufw if missing
#   2. Creates the `sophis` system user + data dirs
#   3. Builds binaries from source (if BUILD_FROM_SOURCE=1) or expects pre-built ones at /tmp
#   4. Installs sophisd-testnet.service + faucet.service
#   5. Generates a faucet wallet if none exists (prints address — fund it manually)
#   6. Configures nginx vhost + obtains TLS cert via certbot
#   7. Configures ufw firewall (22, 80, 443, 46211 testnet P2P; blocks 8181 and 46210 gRPC)
#   8. Enables and starts services

set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────────────
DOMAIN="${DOMAIN:-faucet.sophis.org}"
ACME_EMAIL="${ACME_EMAIL:-ops@sophis.org}"
SOPHIS_USER="${SOPHIS_USER:-sophis}"
SOPHIS_REPO_DIR="${SOPHIS_REPO_DIR:-/opt/sophis}"
SOPHIS_BRANCH="${SOPHIS_BRANCH:-main}"
BUILD_FROM_SOURCE="${BUILD_FROM_SOURCE:-1}"
DRIP_AMOUNT_SOMPI="${DRIP_AMOUNT_SOMPI:-1000000000}"        # 10 SPHS
DRIP_COOLDOWN_SECS="${DRIP_COOLDOWN_SECS:-86400}"           # 24h
# ─────────────────────────────────────────────────────────────────────────────

log() { printf '\033[1;34m[%s]\033[0m %s\n' "$(date -u +%H:%M:%SZ)" "$*"; }
fail() { printf '\033[1;31m[ERROR]\033[0m %s\n' "$*" >&2; exit 1; }

[[ $EUID -ne 0 ]] || fail "Don't run as root — use a sudo-capable user."
sudo -n true 2>/dev/null || fail "sudo with no password prompt required for non-interactive deploy. Run 'sudo -v' first."

log "Sophis testnet faucet deploy → $DOMAIN"

# 1. Packages
log "Step 1/8: install packages"
sudo apt-get update -qq
sudo apt-get install -y -qq nginx certbot python3-certbot-nginx ufw curl jq

# 2. User + dirs
log "Step 2/8: create sophis user + data dirs"
if ! id -u "$SOPHIS_USER" >/dev/null 2>&1; then
    sudo useradd --system --create-home --home-dir "/var/lib/$SOPHIS_USER" --shell /usr/sbin/nologin "$SOPHIS_USER"
fi
sudo install -d -o "$SOPHIS_USER" -g "$SOPHIS_USER" -m 0750 \
    "/var/lib/sophis" \
    "/var/lib/sophis/testnet" \
    "/var/lib/sophis/testnet/logs" \
    "/var/lib/sophis-faucet"

# 3. Build (optional)
if [[ "$BUILD_FROM_SOURCE" == "1" ]]; then
    log "Step 3/8: build binaries from source"
    if [[ ! -d "$SOPHIS_REPO_DIR/.git" ]]; then
        sudo git clone --depth=1 --branch "$SOPHIS_BRANCH" https://github.com/sophis-network/Sophis "$SOPHIS_REPO_DIR"
    else
        sudo git -C "$SOPHIS_REPO_DIR" fetch origin
        sudo git -C "$SOPHIS_REPO_DIR" checkout "$SOPHIS_BRANCH"
        sudo git -C "$SOPHIS_REPO_DIR" pull --ff-only
    fi
    # Cargo build requires: rustup with rust 1.94+, llvm, protoc, cmake — see CLAUDE.md
    sudo apt-get install -y -qq build-essential cmake protobuf-compiler libclang-dev pkg-config libssl-dev
    if ! command -v cargo >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sudo -u "$SOPHIS_USER" sh -s -- -y --default-toolchain stable
    fi
    sudo -u "$SOPHIS_USER" bash -lc "cd $SOPHIS_REPO_DIR && cargo build --release -p sophisd -p testnet-faucet -p dilithium-wallet"
    sudo install -m 0755 "$SOPHIS_REPO_DIR/target/release/sophisd" /usr/local/bin/sophisd
    sudo install -m 0755 "$SOPHIS_REPO_DIR/target/release/testnet-faucet" /usr/local/bin/testnet-faucet
    sudo install -m 0755 "$SOPHIS_REPO_DIR/target/release/dilithium-wallet" /usr/local/bin/dilithium-wallet
else
    log "Step 3/8: skipping build — binaries must already be at /usr/local/bin/{sophisd,testnet-faucet,dilithium-wallet}"
    for b in sophisd testnet-faucet dilithium-wallet; do
        [[ -x "/usr/local/bin/$b" ]] || fail "/usr/local/bin/$b not found. Set BUILD_FROM_SOURCE=1 or copy binaries manually."
    done
fi

# 4. Systemd units
log "Step 4/8: install systemd units"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sudo install -m 0644 "$SCRIPT_DIR/sophisd-testnet.service" /etc/systemd/system/sophisd-testnet.service
sudo install -m 0644 "$SCRIPT_DIR/faucet.service" /etc/systemd/system/faucet.service
sudo systemctl daemon-reload

# 5. Wallet
log "Step 5/8: faucet wallet"
WALLET_FILE="/var/lib/sophis-faucet/wallet.json"
if [[ ! -f "$WALLET_FILE" ]]; then
    log "Generating new faucet wallet — RECORD THE ADDRESS AND FUND IT"
    sudo -u "$SOPHIS_USER" /usr/local/bin/testnet-faucet generate-wallet \
        --wallet "$WALLET_FILE" \
        --network testnet
    sudo chmod 600 "$WALLET_FILE"
    echo
    echo "==============================================="
    echo "  Fund the faucet wallet before starting:"
    sudo jq -r '.address' "$WALLET_FILE" | sudo -u "$SOPHIS_USER" tee /dev/null
    echo "==============================================="
    echo
else
    log "Wallet already exists at $WALLET_FILE — skipping keygen"
fi

# 6. Nginx + TLS
log "Step 6/8: nginx vhost + TLS"
sudo install -m 0644 "$SCRIPT_DIR/nginx-faucet.sophis.org.conf" "/etc/nginx/sites-available/$DOMAIN"
# Replace placeholder if domain differs from default
sudo sed -i "s|faucet.sophis.org|$DOMAIN|g" "/etc/nginx/sites-available/$DOMAIN"
sudo ln -sf "/etc/nginx/sites-available/$DOMAIN" "/etc/nginx/sites-enabled/$DOMAIN"

# Rate limit zones — install once
if [[ ! -f /etc/nginx/conf.d/00-faucet-rate-limits.conf ]]; then
    cat <<'EOF' | sudo tee /etc/nginx/conf.d/00-faucet-rate-limits.conf >/dev/null
limit_req_zone $binary_remote_addr zone=faucet_drip:10m rate=6r/m;
limit_req_zone $binary_remote_addr zone=faucet_browse:10m rate=60r/m;
EOF
fi

sudo nginx -t
sudo systemctl reload nginx

# certbot is idempotent; --keep-until-expiring skips if cert is still valid
sudo certbot --nginx -d "$DOMAIN" \
    --non-interactive --agree-tos --email "$ACME_EMAIL" \
    --keep-until-expiring --redirect --hsts || log "certbot failed — check DNS A record for $DOMAIN and re-run"

# 7. Firewall
log "Step 7/8: ufw firewall"
sudo ufw --force default deny incoming
sudo ufw --force default allow outgoing
sudo ufw allow 22/tcp comment 'SSH'
sudo ufw allow 80/tcp comment 'HTTP (ACME + redirect)'
sudo ufw allow 443/tcp comment 'HTTPS faucet'
sudo ufw allow 46211/tcp comment 'Sophis testnet P2P'
# 46210 (gRPC), 47210 (Borsh), 48210 (JSON), 8181 (faucet upstream) — internal only
sudo ufw --force enable

# 8. Start
log "Step 8/8: enable + start services"
sudo systemctl enable --now sophisd-testnet.service
sleep 5
sudo systemctl enable --now faucet.service

log "Done. Verify:"
echo "  - sudo systemctl status sophisd-testnet faucet"
echo "  - curl https://$DOMAIN/status"
echo "  - tail -f /var/log/nginx/faucet.access.log"
echo
echo "If wallet is unfunded, faucet will return 503 'Faucet has no spendable funds'."
echo "Fund the address printed above, wait for coinbase maturity (1000 blocks ≈ 100min on testnet), then drips work."
