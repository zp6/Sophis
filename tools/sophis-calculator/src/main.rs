//! H1 — Sophis Energy Offset Calculator local dev server.
//!
//! Tiny axum binary that serves the static HTML/JS/CSS in
//! `tools/sophis-calculator/static/` for local development. The
//! production deployment is the same three static files dropped into
//! a CDN / GitHub Pages — this binary is purely a developer
//! convenience so contributors can `cargo run -p sophis-calculator`
//! and view the page at http://localhost:46410.
//!
//! Mirrors the embed-via-`include_str!` pattern from
//! `tools/sophis-dashboard`. No RPC, no consensus access, no state.

use std::net::SocketAddr;

use axum::{Router, http::header, response::IntoResponse, routing::get};
use clap::{Arg, Command};
use log::info;

const EMBEDDED_HTML: &str = include_str!("../static/index.html");
const EMBEDDED_JS: &str = include_str!("../static/app.js");
const EMBEDDED_CSS: &str = include_str!("../static/style.css");

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:46410";

async fn root() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], EMBEDDED_HTML)
}

async fn app_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript; charset=utf-8")], EMBEDDED_JS)
}

async fn style_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], EMBEDDED_CSS)
}

async fn healthz() -> impl IntoResponse {
    "ok"
}

#[tokio::main]
async fn main() {
    sophis_core::log::init_logger(None, "info");

    let m = Command::new("sophis-calculator")
        .about("Sophis Energy Offset Calculator — local dev server")
        .arg(
            Arg::new("listen-addr")
                .long("listen-addr")
                .short('l')
                .default_value(DEFAULT_LISTEN_ADDR)
                .help("Bind address for the HTTP server"),
        )
        .get_matches();

    let listen_addr_str = m.get_one::<String>("listen-addr").unwrap();
    let listen_addr: SocketAddr = listen_addr_str.parse().unwrap_or_else(|e| {
        eprintln!("Erro: --listen-addr inválido: {}", e);
        std::process::exit(1);
    });

    let app = Router::new()
        .route("/", get(root))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/healthz", get(healthz));

    let listener = tokio::net::TcpListener::bind(&listen_addr).await.expect("bind");
    info!("sophis-calculator serving on http://{}", listen_addr);
    axum::serve(listener, app).await.expect("serve");
}
