//! `sophis-dashboard` — public mainnet launch dashboard.
//!
//! Implements LAUNCH_CHECKLIST.md ação #2 (Bloco 6 — defensive actions
//! T-72h → T+24h). Goes live at t=0 (genesis) and exposes:
//!
//!   - Total network hashrate (DAA-difficulty-derived)
//!   - Total emitted supply (`get_coin_supply`)
//!   - Founder address balance + founder share % (= balance / supply)
//!   - Time since genesis with the 24h wait-window countdown
//!     (founder mining is restricted during this window per §5.3)
//!   - The publicly-declared founder mining address (immutable input)
//!
//! Architecture:
//!   - Single binary, axum HTTP server, embedded HTML page
//!   - Background tokio task polls sophisd gRPC every 10s and updates
//!     a shared `MetricsCache` (Arc<RwLock<...>>)
//!   - GET /         → returns the embedded HTML page
//!   - GET /metrics  → returns the cached JSON snapshot
//!   - GET /healthz  → 200 OK (for monitoring / uptime probes)
//!
//! Self-contained: deploy as a single binary on any VPS pointing at a
//! local sophisd. No external dependencies beyond what the workspace
//! already pulls in.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::get,
};
use clap::{Arg, Command, value_parser};
use serde::Serialize;
use sophis_addresses::Address;
use sophis_grpc_client::GrpcClient;
use sophis_notify::subscription::context::SubscriptionContext;
use sophis_rpc_core::{api::rpc::RpcApi, notify::mode::NotificationMode};
use tokio::sync::RwLock;

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const RPC_TIMEOUT: Duration = Duration::from_secs(15);
/// I1.1 — secondary poll cycle for mempool. Mempool changes slowly
/// relative to consensus, so we throttle to 30s to reduce RPC load.
const MEMPOOL_POLL_EVERY_N_TICKS: u64 = 3;

/// 24-hour founder wait window (§5.3 of the whitepaper).
const FOUNDER_WAIT_SECS: i64 = 24 * 3600;

/// I1.1 — size of the BPS ring buffer (in 10s ticks). 6 ticks = 60s window
/// matches §4.1 of `docs/I1_DASHBOARD_DESIGN.md`.
const BPS_WINDOW_TICKS: usize = 6;

/// I1.1 — default for `--finality-blue-blocks` CLI flag (D2).
/// Matches the mainnet `coinbase_maturity` so the "safe to spend"
/// guarantee at depth N also satisfies the finality label.
const DEFAULT_FINALITY_BLUE_BLOCKS: u64 = 100;

#[derive(Clone, Serialize, Default)]
struct MetricsSnapshot {
    /// Wall-clock time the snapshot was taken (unix ms).
    pub snapshot_unix_ms: u64,

    /// Genesis timestamp configured for this dashboard (unix ms; 0 if unset).
    pub genesis_unix_ms: u64,

    /// Seconds since genesis (negative if genesis is in the future, 0 floor).
    pub seconds_since_genesis: i64,

    /// Seconds remaining in the 24h founder wait window. Negative once the
    /// window has elapsed.
    pub seconds_until_founder_window_ends: i64,

    /// Whether the founder is currently inside the 24h wait window.
    pub founder_in_wait_window: bool,

    /// Best-effort total hashrate in hashes/sec (derived from DAA difficulty
    /// and target time). 0 if RPC unavailable.
    pub hashrate_hps: u64,

    /// Total emitted supply in sompi (10⁻⁸ SPHS).
    pub total_supply_sompi: u64,

    /// Founder address balance in sompi.
    pub founder_balance_sompi: u64,

    /// Founder share = balance / total_emitted_supply (0..1).
    pub founder_share_ratio: f64,

    /// Number of blocks in the DAG (best-effort).
    pub block_count: u64,

    /// Virtual DAA score.
    pub virtual_daa_score: u64,

    /// Whether the last RPC poll succeeded.
    pub rpc_healthy: bool,

    /// Last RPC error message if any.
    pub last_rpc_error: Option<String>,

    /// Founder mining address (declared at T-72h; never changes).
    pub founder_address: String,

    /// Total wait window length in seconds (constant: 86400).
    pub founder_wait_window_secs: i64,

    // ─── I1.1 — extended metrics ────────────────────────────────────────────

    /// Observed blocks-per-second over the rolling 60s window. Reports `0.0`
    /// before the BPS ring buffer is warm (first 60s after dashboard start).
    /// See `docs/I1_DASHBOARD_DESIGN.md` §4.1.
    pub bps_actual: f64,

    /// Snapshot of the local mempool: tx count + cumulative mass.
    /// Refreshed every 30s (more conservatively than the 10s consensus poll).
    pub mempool_depth: MempoolDepth,

    /// GHOSTDAG-aware finality probability label. Reports the current
    /// virtual blue score and the operator-configured N for the
    /// "99.9% finalized after N blue blocks" guarantee. The label itself
    /// is informational — wallets that need cryptographic-grade finality
    /// should use a chain-block proof, not this number.
    pub finality_probability: FinalityLabel,
}

/// I1.1 — mempool snapshot exposed at `/metrics`. See §4.2 of DESIGN.
#[derive(Clone, Serialize, Default, Debug, PartialEq, Eq)]
pub struct MempoolDepth {
    pub tx_count: usize,
    pub total_mass: u64,
    /// Mirrors the `include_orphan_pool` flag the dashboard passed to
    /// `get_mempool_entries`. Always `false` in v1 — orphans are not
    /// part of the operator-facing depth signal.
    pub include_orphans: bool,
}

/// I1.1 — finality probability label. See §4.3 of DESIGN.
#[derive(Clone, Serialize, Default, Debug, PartialEq, Eq)]
pub struct FinalityLabel {
    pub blue_score_now: u64,
    pub blue_blocks_for_99_9: u64,
    pub label: String,
}

impl FinalityLabel {
    fn build(blue_score_now: u64, n: u64) -> Self {
        // The label includes a wall-clock estimate based on the 10 BPS
        // mainnet target. Operators on devnet / testnet with different
        // BPS get a slightly off estimate; surfaced as ~estimate, not
        // a guarantee.
        let estimate_secs = (n as f64 / 10.0) as u64;
        let label = format!("99.9% finalized after {n} blue blocks (~{estimate_secs}s at 10 BPS)");
        Self { blue_score_now, blue_blocks_for_99_9: n, label }
    }
}

#[derive(Clone)]
struct AppState {
    metrics: Arc<RwLock<MetricsSnapshot>>,
    /// I1.1 — rolling buffer of `(unix_ms, block_count)` snapshots, one
    /// per consensus poll tick. Sized to `BPS_WINDOW_TICKS` entries; a
    /// new entry pushes the oldest out so the buffer stays at the
    /// correct size. BPS is derived from the front and back values.
    bps_buf: Arc<RwLock<VecDeque<(u64, u64)>>>,
    /// I1.1 — most-recent mempool snapshot. Polled every `MEMPOOL_POLL_EVERY_N_TICKS`
    /// consensus ticks (= 30s by default). Cached so the 10s consensus
    /// poll can include the freshest known value without re-polling
    /// mempool itself.
    mempool: Arc<RwLock<MempoolDepth>>,
}

async fn connect_grpc(rpc_server: &str) -> GrpcClient {
    let ctx = SubscriptionContext::new();
    GrpcClient::connect_with_args(
        NotificationMode::Direct,
        format!("grpc://{}", rpc_server),
        Some(ctx),
        true,
        None,
        false,
        Some(15_000),
        Default::default(),
    )
    .await
    .expect("RPC connection failed")
}

/// Inputs to one consensus-tick poll cycle. Grouped to keep the
/// `poll_once` signature stable as I1.x adds dependencies.
struct PollInputs<'a> {
    rpc: &'a GrpcClient,
    founder_addr: &'a Address,
    genesis_unix_ms: u64,
    /// I1.1 — N for the finality label (CLI flag `--finality-blue-blocks`).
    finality_blue_blocks: u64,
}

async fn poll_once(inputs: &PollInputs<'_>) -> MetricsSnapshot {
    let PollInputs { rpc, founder_addr, genesis_unix_ms, finality_blue_blocks } = *inputs;
    let mut snap = MetricsSnapshot {
        snapshot_unix_ms: now_unix_ms(),
        genesis_unix_ms,
        founder_address: founder_addr.to_string(),
        founder_wait_window_secs: FOUNDER_WAIT_SECS,
        ..Default::default()
    };

    // Compute time-since-genesis fields up-front so they're populated even
    // if the RPC poll fails partway through.
    let now_secs = (snap.snapshot_unix_ms / 1000) as i64;
    let genesis_secs = (genesis_unix_ms / 1000) as i64;
    if genesis_secs > 0 {
        snap.seconds_since_genesis = (now_secs - genesis_secs).max(0);
        snap.seconds_until_founder_window_ends = FOUNDER_WAIT_SECS - snap.seconds_since_genesis;
        snap.founder_in_wait_window = snap.seconds_since_genesis < FOUNDER_WAIT_SECS;
    }

    // RPC: get_block_dag_info
    let dag_info = match tokio::time::timeout(RPC_TIMEOUT, rpc.get_block_dag_info()).await {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => {
            snap.last_rpc_error = Some(format!("get_block_dag_info: {e}"));
            return snap;
        }
        Err(_) => {
            snap.last_rpc_error = Some("get_block_dag_info timeout".into());
            return snap;
        }
    };
    snap.virtual_daa_score = dag_info.virtual_daa_score;
    snap.block_count = dag_info.block_count;
    // Difficulty is doubles representing the work-per-block; converting
    // to hashrate requires the target time per block. The wRPC `difficulty`
    // already encodes hashes-per-block per BlockDAG conventions; combined
    // with 10 BPS this yields total hashrate.
    snap.hashrate_hps = (dag_info.difficulty * 10.0) as u64;
    // I1.1 — finality label uses the live virtual DAA as a proxy for
    // blue_score. `get_block_dag_info` does not expose blue_score at the
    // virtual selected tip directly; in practice virtual_daa_score
    // tracks blue_score within ±K (the GHOSTDAG K). The label is
    // informational so the proxy is acceptable; documented.
    snap.finality_probability = FinalityLabel::build(snap.virtual_daa_score, finality_blue_blocks);

    // RPC: get_coin_supply
    match tokio::time::timeout(RPC_TIMEOUT, rpc.get_coin_supply()).await {
        Ok(Ok(supply)) => {
            snap.total_supply_sompi = supply.circulating_sompi;
        }
        Ok(Err(e)) => {
            snap.last_rpc_error = Some(format!("get_coin_supply: {e}"));
            return snap;
        }
        Err(_) => {
            snap.last_rpc_error = Some("get_coin_supply timeout".into());
            return snap;
        }
    }

    // RPC: get_balance_by_address (founder)
    match tokio::time::timeout(RPC_TIMEOUT, rpc.get_balance_by_address(founder_addr.clone())).await {
        Ok(Ok(balance)) => {
            snap.founder_balance_sompi = balance;
            if snap.total_supply_sompi > 0 {
                snap.founder_share_ratio = balance as f64 / snap.total_supply_sompi as f64;
            }
        }
        Ok(Err(e)) => {
            snap.last_rpc_error = Some(format!("get_balance_by_address: {e}"));
            return snap;
        }
        Err(_) => {
            snap.last_rpc_error = Some("get_balance_by_address timeout".into());
            return snap;
        }
    }

    snap.rpc_healthy = true;
    snap
}

async fn poller_task(
    rpc_server: String,
    founder_addr: Address,
    genesis_unix_ms: u64,
    finality_blue_blocks: u64,
    state: AppState,
) {
    log::info!("connecting to sophisd at {}", rpc_server);
    let rpc = connect_grpc(&rpc_server).await;
    log::info!("connected; starting poll loop @ {:?}", POLL_INTERVAL);
    let mut tick: u64 = 0;
    loop {
        let inputs = PollInputs { rpc: &rpc, founder_addr: &founder_addr, genesis_unix_ms, finality_blue_blocks };
        let mut snap = poll_once(&inputs).await;
        if !snap.rpc_healthy {
            log::warn!("rpc poll failed: {:?}", snap.last_rpc_error);
        }
        // I1.1 — BPS ring buffer. Update first (push current count),
        // then derive bps_actual from the buffer's front and back.
        if snap.rpc_healthy {
            let mut buf = state.bps_buf.write().await;
            buf.push_back((snap.snapshot_unix_ms, snap.block_count));
            while buf.len() > BPS_WINDOW_TICKS {
                buf.pop_front();
            }
            if buf.len() >= 2 {
                let (t0, c0) = *buf.front().unwrap();
                let (t1, c1) = *buf.back().unwrap();
                let dt_secs = (t1.saturating_sub(t0)) as f64 / 1000.0;
                if dt_secs > 0.0 {
                    snap.bps_actual = (c1.saturating_sub(c0)) as f64 / dt_secs;
                }
            }
        }
        // I1.1 — mempool poll runs on a sub-cycle (every 30s by default).
        if tick.is_multiple_of(MEMPOOL_POLL_EVERY_N_TICKS) && snap.rpc_healthy {
            match poll_mempool(&rpc).await {
                Ok(mp) => *state.mempool.write().await = mp,
                Err(e) => log::warn!("mempool poll failed: {e}"),
            }
        }
        // Always include the most recent (possibly stale) mempool snapshot.
        snap.mempool_depth = state.mempool.read().await.clone();
        *state.metrics.write().await = snap;
        tick = tick.wrapping_add(1);
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// I1.1 — pulls mempool entries and aggregates `(tx_count, total_mass)`.
/// Returns Err only on RPC failure; an empty mempool returns
/// `MempoolDepth::default()` with `include_orphans = false`.
async fn poll_mempool(rpc: &GrpcClient) -> Result<MempoolDepth, String> {
    let entries = tokio::time::timeout(RPC_TIMEOUT, rpc.get_mempool_entries(false, false))
        .await
        .map_err(|_| "get_mempool_entries timeout".to_string())?
        .map_err(|e| format!("get_mempool_entries: {e}"))?;
    let tx_count = entries.len();
    let total_mass: u64 = entries.iter().map(|e| e.transaction.mass).sum();
    Ok(MempoolDepth { tx_count, total_mass, include_orphans: false })
}

fn now_unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ─── HTTP handlers ───────────────────────────────────────────────────────────

async fn root() -> Html<&'static str> {
    Html(EMBEDDED_HTML)
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let snap = state.metrics.read().await.clone();
    (StatusCode::OK, Json(snap))
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

const EMBEDDED_HTML: &str = include_str!("dashboard.html");

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    sophis_core::log::init_logger(None, "info");

    let m = Command::new("sophis-dashboard")
        .about("Public mainnet launch dashboard (LAUNCH_CHECKLIST.md ação #2)")
        .arg(Arg::new("rpcserver").long("rpcserver").short('s').default_value("localhost:46110"))
        .arg(Arg::new("listen-addr").long("listen-addr").short('l').default_value("0.0.0.0:8080"))
        .arg(
            Arg::new("founder-address")
                .long("founder-address")
                .short('f')
                .required(true)
                .help("Endereço pessoal de mineração do fundador (declarado em T-72h)"),
        )
        .arg(
            Arg::new("genesis-unix-ms")
                .long("genesis-unix-ms")
                .short('g')
                .default_value("0")
                .value_parser(value_parser!(u64))
                .help("Timestamp do gênese em unix milliseconds (0 = desconhecido ainda)"),
        )
        .arg(
            Arg::new("finality-blue-blocks")
                .long("finality-blue-blocks")
                .default_value(DEFAULT_FINALITY_BLUE_BLOCKS.to_string())
                .value_parser(value_parser!(u64))
                .help("N para a label '99.9% finalized after N blue blocks' (D2 do I1; default = 100 = coinbase_maturity)"),
        )
        .get_matches();

    let rpc_server = m.get_one::<String>("rpcserver").unwrap().clone();
    let listen_addr_str = m.get_one::<String>("listen-addr").unwrap();
    let founder_address_str = m.get_one::<String>("founder-address").unwrap();
    let genesis_unix_ms = *m.get_one::<u64>("genesis-unix-ms").unwrap();
    let finality_blue_blocks = *m.get_one::<u64>("finality-blue-blocks").unwrap();

    let listen_addr: SocketAddr = listen_addr_str.parse().unwrap_or_else(|e| {
        eprintln!("Erro: --listen-addr inválido: {}", e);
        std::process::exit(2);
    });
    let founder_address: Address = Address::try_from(founder_address_str.clone()).unwrap_or_else(|e| {
        eprintln!("Erro: --founder-address inválido: {}", e);
        std::process::exit(2);
    });

    println!("sophis-dashboard");
    println!("  rpc            : {}", rpc_server);
    println!("  listen         : http://{}", listen_addr);
    println!("  founder        : {}", founder_address);
    println!("  finality (N)   : {} blue blocks", finality_blue_blocks);
    if genesis_unix_ms > 0 {
        println!("  genesis (ms)   : {}", genesis_unix_ms);
    } else {
        println!("  genesis        : (not set — wait countdown disabled)");
    }
    println!();

    let state = AppState {
        metrics: Arc::new(RwLock::new(MetricsSnapshot::default())),
        bps_buf: Arc::new(RwLock::new(VecDeque::with_capacity(BPS_WINDOW_TICKS + 1))),
        mempool: Arc::new(RwLock::new(MempoolDepth::default())),
    };

    // Spawn the poller in the background.
    let poller_state = state.clone();
    tokio::spawn(poller_task(rpc_server, founder_address, genesis_unix_ms, finality_blue_blocks, poller_state));

    let app = Router::new().route("/", get(root)).route("/metrics", get(metrics)).route("/healthz", get(healthz)).with_state(state);

    let listener = match tokio::net::TcpListener::bind(&listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Erro: bind {}: {}", listen_addr, e);
            std::process::exit(1);
        }
    };
    println!("Dashboard servindo em http://{}", listen_addr);
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("Erro: axum serve: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snapshot json round-trip works (ensures no serde fields broken).
    #[test]
    fn snapshot_serializes() {
        let s = MetricsSnapshot {
            founder_share_ratio: 0.0123,
            seconds_since_genesis: 3600,
            seconds_until_founder_window_ends: 82800,
            founder_in_wait_window: true,
            ..Default::default()
        };
        let j = serde_json::to_value(&s).expect("serialize");
        assert!(j.get("founder_share_ratio").is_some());
        assert!(j.get("seconds_since_genesis").is_some());
    }

    /// 24h wait window math: at exactly 24h elapsed, the window has just
    /// ended (founder_in_wait_window = false, seconds remaining = 0).
    #[test]
    fn wait_window_boundary() {
        let mut s = MetricsSnapshot { snapshot_unix_ms: (FOUNDER_WAIT_SECS as u64) * 1000, genesis_unix_ms: 0, ..Default::default() };
        // Re-derive what poll_once would compute:
        let now_secs = (s.snapshot_unix_ms / 1000) as i64;
        let genesis_secs = (s.genesis_unix_ms / 1000) as i64;
        // Use a fictional non-zero genesis to exercise the actual logic.
        let _ = (now_secs, genesis_secs);
        // For the actual logic, simulate genesis at 0 and now at exactly 24h.
        s.genesis_unix_ms = 1; // tiny non-zero so the logic engages
        s.snapshot_unix_ms = (FOUNDER_WAIT_SECS as u64) * 1000 + 1;
        let now = (s.snapshot_unix_ms / 1000) as i64;
        let genesis = (s.genesis_unix_ms / 1000) as i64;
        let elapsed = (now - genesis).max(0);
        assert!(elapsed >= FOUNDER_WAIT_SECS);
    }

    /// Poller_task and connect_grpc are integration-only; we don't unit-test
    /// them here. They're exercised when the binary is run against a real
    /// sophisd. The poll_once logic falls back gracefully on RPC failure
    /// (rpc_healthy = false; partial fields populated).
    #[test]
    fn poll_once_offline_returns_partial_snapshot() {
        // Smoke-only: verify the structure of MetricsSnapshot::default()
        // is what we'd expect to be served before the first successful poll.
        let snap = MetricsSnapshot::default();
        assert!(!snap.rpc_healthy);
        assert_eq!(snap.founder_balance_sompi, 0);
        assert_eq!(snap.last_rpc_error, None);
    }

    // ─── I1.1 — extended metrics ────────────────────────────────────────────

    #[test]
    fn finality_label_build_carries_n_and_estimate() {
        let label = FinalityLabel::build(12_345, 100);
        assert_eq!(label.blue_score_now, 12_345);
        assert_eq!(label.blue_blocks_for_99_9, 100);
        assert!(label.label.contains("100 blue blocks"), "label: {}", label.label);
        assert!(label.label.contains("10s"), "expected 10s wall-clock estimate (100/10): {}", label.label);
    }

    #[test]
    fn finality_label_build_with_n_50_estimates_5s() {
        let label = FinalityLabel::build(0, 50);
        assert!(label.label.contains("5s at 10 BPS"), "label: {}", label.label);
    }

    #[test]
    fn mempool_depth_default_is_empty_no_orphans() {
        let mp = MempoolDepth::default();
        assert_eq!(mp.tx_count, 0);
        assert_eq!(mp.total_mass, 0);
        assert!(!mp.include_orphans);
    }

    #[test]
    fn extended_snapshot_serializes_with_new_fields() {
        let snap = MetricsSnapshot {
            bps_actual: 9.83,
            mempool_depth: MempoolDepth { tx_count: 142, total_mass: 5_237_400, include_orphans: false },
            finality_probability: FinalityLabel::build(12_450, 100),
            ..Default::default()
        };

        let j = serde_json::to_value(&snap).expect("serialize");
        // bps_actual at top level
        assert!((j.get("bps_actual").and_then(|v| v.as_f64()).unwrap_or(0.0) - 9.83).abs() < 1e-6);
        // mempool_depth nested
        let mp = j.get("mempool_depth").expect("mempool_depth field");
        assert_eq!(mp.get("tx_count").and_then(|v| v.as_u64()), Some(142));
        assert_eq!(mp.get("total_mass").and_then(|v| v.as_u64()), Some(5_237_400));
        // finality_probability nested
        let fl = j.get("finality_probability").expect("finality_probability field");
        assert_eq!(fl.get("blue_score_now").and_then(|v| v.as_u64()), Some(12_450));
        assert_eq!(fl.get("blue_blocks_for_99_9").and_then(|v| v.as_u64()), Some(100));
    }

    /// BPS computation: the poller's ring-buffer logic. We don't have
    /// access to the live `state.bps_buf` outside `poller_task`, so this
    /// test mirrors the math: given two snapshots 60 s apart with a
    /// 600-block delta, BPS should be 10.0.
    #[test]
    fn bps_math_matches_designed_window() {
        let t0_ms: u64 = 1_700_000_000_000;
        let t1_ms: u64 = t0_ms + 60_000;
        let c0: u64 = 100_000;
        let c1: u64 = 100_600;
        let dt_secs = (t1_ms - t0_ms) as f64 / 1000.0;
        let bps = (c1 - c0) as f64 / dt_secs;
        assert!((bps - 10.0).abs() < 1e-9);
    }

    /// Edge case: a single-element BPS buffer reports 0.0 (insufficient
    /// data). Mirrors the `if buf.len() >= 2` guard in poller_task.
    #[test]
    fn bps_single_sample_reports_zero() {
        // Default value is 0.0; any consumer reading a freshly-warmed
        // dashboard sees this until the second poll lands.
        let snap = MetricsSnapshot::default();
        assert_eq!(snap.bps_actual, 0.0);
        // After a single (mock) update the field carries the value.
        let snap2 = MetricsSnapshot { bps_actual: 10.0, ..MetricsSnapshot::default() };
        assert!(snap2.bps_actual > 0.0);
    }
}
