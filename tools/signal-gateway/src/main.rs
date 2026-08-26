//! Signal Gateway — a lightweight Signal daemon edge.
//!
//! Switchboard (brain-server v1.28.43) role: this process is the GOVERNED
//! EDGE for the Signal channel. It holds ONLY its own credentials (the
//! presage Signal store + its 0600 config) and NEVER brain-server tokens or
//! database access; the kernel stays channel-free by construction.
//!
//! Core fixes carried from the original implementation:
//! 1. Bounded channels 2. Command loop 3. compare_exchange receiver spawn
//! 4. Graceful shutdown 5. WorkerState enum 6. Semaphore rate limiting
//! 7. Input validation 8. Oneshot timeouts 9. No hardcoded identity
//!
//! Memory safety: the crate is `#![forbid(unsafe_code)]` — enforced by the
//! compiler, not convention.
#![forbid(unsafe_code)]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod brain;
mod cache;
mod config;
mod ratelimit;
mod signal;
mod state;
mod validation;

use config::Config;
use state::AppState;

#[derive(Parser)]
#[command(name = "signal-gateway")]
#[command(about = "Lightweight Signal daemon edge", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve {
        #[arg(short, long, default_value = "config.yaml")]
        config: PathBuf,
    },
    Link {
        #[arg(short, long, default_value = "config.yaml")]
        config: PathBuf,
        #[arg(long)]
        device_name: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(tracing_subscriber::fmt::layer().compact())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { config } => {
            let config = Config::load(&config)?;
            info!("Starting Signal Gateway v{}", env!("CARGO_PKG_VERSION"));

            let state: AppState = AppState::new(config.clone())?;

            // Try to load registered account
            match state.init_signal().await {
                Ok(true) => {
                    if let Ok(Some(number)) = state.signal.get_profile().await {
                        info!("Signal linked: {}", number);
                    } else {
                        info!("Signal linked");
                    }
                }
                Ok(false) => {
                    info!("Signal not linked. Use 'link' command to pair.");
                }
                Err(e) => {
                    info!("Signal init error: {}. Use 'link' command.", e);
                }
            }

            // Switchboard seam: absent brain config = channel-dark edge.
            if let Some(brain) = &config.brain {
                state.start_brain_adapter(brain);
            } else {
                info!("no brain config — running channel-dark");
            }

            // Privacy posture: refuse non-loopback binds unless explicitly
            // overridden — this API controls a live Signal identity.
            let addr: std::net::SocketAddr = config
                .server
                .address
                .parse()
                .context("server.address must be ip:port")?;
            if !addr.ip().is_loopback()
                && std::env::var("SIGNAL_GATEWAY_ALLOW_REMOTE").as_deref() != Ok("1")
            {
                anyhow::bail!(
                    "refusing to bind {} (not loopback) without SIGNAL_GATEWAY_ALLOW_REMOTE=1",
                    addr
                );
            }

            let app = if let Some(token) = &config.server.auth_token {
                info!("API auth: bearer token required");
                api::create_router_with_auth(state, Some(token.clone()))
            } else {
                info!("API auth: NONE (loopback-only posture)");
                api::create_router(state)
            };
            let listener = tokio::net::TcpListener::bind(addr).await?;
            info!("Listening on {}", addr);
            info!("Endpoints:");
            info!("  GET  /v1/health       - Health check");
            info!("  GET  /v1/about        - Account info");
            info!("  GET  /api/v1/events   - SSE message stream");
            info!("  POST /api/v1/rpc      - JSON-RPC API");

            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .ok();
        }
        Commands::Link {
            config,
            device_name,
        } => {
            let config = Config::load(&config)?;
            let state: AppState = AppState::new(config.clone())?;

            let signal = state.signal.clone();
            let device_name = device_name.unwrap_or_else(|| "signal-gateway".to_string());

            info!("Generating link URL for device: {}", device_name);
            match signal.link_secondary_device(device_name).await {
                Ok(url) => {
                    info!("Scan this QR code with your Signal app:");
                    info!("");
                    info!("{}", url);
                    info!("");
                    info!("Or open the URL in your browser");
                }
                Err(e) => {
                    tracing::error!("Failed to generate link URL: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            tracing::error!("failed to install Ctrl+C handler");
            std::process::exit(1);
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!("failed to install SIGTERM handler: {e}");
                std::process::exit(1);
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C"),
        _ = terminate => info!("Received SIGTERM"),
    }

    info!("Shutting down gracefully...");
}
