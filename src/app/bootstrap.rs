use std::sync::Arc;

use log::LevelFilter;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{signature::Keypair, signer::Signer};
use tokio::{
    fs::File,
    io::AsyncReadExt,
    sync::{mpsc, watch},
};

use crate::{
    adapters::{
        fpga_feed::FpgaFeedAdapter, network_path::NetworkPathProfile,
        solana_logs::SolanaPubsubLogStream, toml_rules::TomlRuleRepository,
    },
    app::{
        context::ExecutionContext, logging::init_logging, systemd::maybe_handle_service_command,
    },
    domain::{settings::RuntimeSettings, value_objects::sol_amount::Lamports},
    ports::{fpga_feed::FpgaFeedPort, log_stream::LogStreamPort, network_path::NetworkPathPort},
    slices::{
        config_sync::service::{ConfigSyncService, load_rulebook},
        sniper::{
            engine::SniperEngine,
            replay::{log_replay_report, run_synthetic_replay},
            telemetry::LatencyTelemetry,
        },
    },
};

pub async fn run() {
    if let Err(error) = run_inner().await {
        eprintln!("{}", error);
        std::process::exit(1);
    }
}

async fn run_inner() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if maybe_handle_service_command(&args)? {
        return Ok(());
    }

    init_logging(resolve_level_filter()).await?;

    log::info!("Slotstrike runtime");

    let settings = RuntimeSettings::from_cli_args(&args)?;

    if settings.run_replay_benchmark {
        let report = run_synthetic_replay(
            settings.replay_event_count.get(),
            settings.replay_burst_size.get(),
        );
        log_replay_report(&report);
        return Ok(());
    }

    let keypair = Arc::new(load_keypair(&settings.keypair_path).await?);
    let rpc = Arc::new(RpcClient::new(settings.rpc_url.clone()));

    let network_path = NetworkPathProfile::from_settings(&settings);
    let fpga_feed = FpgaFeedAdapter::new(
        settings.fpga_vendor.as_str().to_owned(),
        settings.fpga_verbose,
    );

    let repository = Arc::new(TomlRuleRepository::new(settings.config_path.clone()));
    let initial_rulebook = load_rulebook(repository.as_ref(), true)
        .await
        .map_err(|error| format!("Failed to read rules: {}", error))?;

    let (rulebook_tx, rulebook_rx) = watch::channel(Arc::clone(&initial_rulebook));

    let config_sync_service = ConfigSyncService::new(
        Arc::clone(&repository),
        rulebook_tx,
        Arc::clone(&initial_rulebook),
    );
    config_sync_service.spawn();

    let balance = rpc
        .get_balance(&keypair.pubkey())
        .await
        .map(|lamports| Lamports::new(lamports).as_sol_string())
        .map_err(|error| format!("Failed to read wallet balance: {}", error))?;

    let mint_rules = initial_rulebook.mint_log_lines();
    let deployer_rules = initial_rulebook.deployer_log_lines();
    let mints_string = format_rules(&mint_rules);
    let deployers_string = format_rules(&deployer_rules);

    log::info!(
        "Settings: \
\n\tWallet: {}\
\n\tWallet Balance: {} SOL\
\n\tPRIORITY_FEES: {} µLamports\
\n\tMINTS:\
\t\t{}\
\n\tDEPLOYERS:\
\t\t{}\
\n\tTX_SUBMISSION_MODE: {}\
\n\tJITO_URL: {}\
\n\tRPC_URL: {}\
\n\tWSS_URL: {}\
\n\tNETWORK_STACK_MODE: {}\
\n\tNETWORK_PATH: {}\
\n\tKERNEL_TCP_BYPASS: {}\
\n\tFPGA_ENABLED: {}\
\n\tTELEMETRY_ENABLED: {}",
        keypair.pubkey(),
        balance,
        settings.priority_fees.as_u64(),
        mints_string,
        deployers_string,
        settings.tx_submission_mode.as_str(),
        settings.jito_url,
        settings.rpc_url,
        settings.wss_url.as_str(),
        network_path.mode().as_str(),
        network_path.describe(),
        network_path.kernel_bypass_enabled(),
        network_path.fpga_enabled(),
        settings.telemetry_enabled,
    );

    if settings.fpga_enabled {
        log::info!(
            "FPGA_FEED: {} (vendor={}, verbose={})",
            fpga_feed.describe(),
            fpga_feed.vendor(),
            fpga_feed.verbose(),
        );
    }

    let context = Arc::new(ExecutionContext {
        priority_fees: settings.priority_fees.as_u64(),
        rpc,
        keypair,
        tx_submission_mode: settings.tx_submission_mode,
        jito_url: Arc::new(settings.jito_url.clone()),
    });

    let telemetry = Arc::new(if settings.telemetry_enabled {
        LatencyTelemetry::new(settings.latency_sample_capacity, settings.latency_slo_ns)
    } else {
        LatencyTelemetry::disabled()
    });
    Arc::clone(&telemetry).spawn_reporter(std::time::Duration::from_secs(
        settings.latency_report_period_secs,
    ));

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let kernel_bypass_stream = SolanaPubsubLogStream::kernel_bypass(
        settings.wss_url.as_str().to_owned(),
        settings.kernel_tcp_bypass_engine,
        settings.kernel_bypass_socket_path.as_str().to_owned(),
    );
    let standard_tcp_stream =
        SolanaPubsubLogStream::standard_tcp(settings.wss_url.as_str().to_owned());

    let mut stream_started = false;

    if settings.fpga_enabled {
        match fpga_feed.spawn_stream(events_tx.clone()) {
            Ok(()) => {
                log::info!(
                    "Ingress path selected: FPGA DMA ring -> strategy events (zero-copy frame parse)"
                );
                stream_started = true;
            }
            Err(error) => {
                log::warn!(
                    "FPGA ingress unavailable: {}. Continuing failover chain.",
                    error
                );
            }
        }
    }

    if !stream_started && network_path.kernel_bypass_enabled() {
        match kernel_bypass_stream.spawn_stream(events_tx.clone()) {
            Ok(()) => {
                log::info!(
                    "Ingress path selected: {} -> strategy events",
                    kernel_bypass_stream.path_name()
                );
                stream_started = true;
            }
            Err(error) => {
                log::warn!(
                    "Kernel bypass ingress unavailable: {}. Falling back to standard tcp path.",
                    error
                );
            }
        }
    }

    if !stream_started {
        standard_tcp_stream
            .spawn_stream(events_tx)
            .map_err(|error| {
                format!(
                    "Failed to start {} ingress: {}",
                    standard_tcp_stream.path_name(),
                    error
                )
            })?;
        log::info!(
            "Ingress path selected: {} -> strategy events",
            standard_tcp_stream.path_name()
        );
        stream_started = true;
    }

    if !stream_started {
        return Err("No ingress path could be started".to_owned());
    }

    let engine = SniperEngine::new(context, events_rx, rulebook_rx, telemetry);
    engine.run().await;

    Ok(())
}

async fn load_keypair(path: &str) -> Result<Keypair, String> {
    let mut keypair_file = File::open(path)
        .await
        .map_err(|error| format!("Failed to open keypair file: {}", error))?;

    let mut contents = String::new();
    keypair_file
        .read_to_string(&mut contents)
        .await
        .map_err(|error| format!("Failed to read keypair file: {}", error))?;

    let keypair_bytes = serde_json::from_str::<Vec<u8>>(&contents)
        .map_err(|error| format!("Failed to parse keypair json: {}", error))?;

    Keypair::try_from(keypair_bytes.as_slice())
        .map_err(|error| format!("Invalid keypair bytes: {}", error))
}

fn resolve_level_filter() -> LevelFilter {
    match std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "info".to_owned())
        .to_lowercase()
        .as_str()
    {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => LevelFilter::Info,
    }
}

fn format_rules(rules: &[String]) -> String {
    if rules.is_empty() {
        "(none)".to_owned()
    } else {
        rules.join("\n\t\t")
    }
}
