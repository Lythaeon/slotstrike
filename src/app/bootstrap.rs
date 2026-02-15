use std::{fmt::Write as _, io::IsTerminal, path::PathBuf, sync::Arc};

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
        context::ExecutionContext,
        errors::{
            AppError, IngressStartupError, KeypairLoadError, RulebookLoadError, WalletBalanceError,
        },
        logging::init_logging,
        readiness::validate_ingress_readiness,
        systemd::maybe_handle_service_command,
    },
    domain::{
        events::RawLogEvent,
        settings::{NetworkStackMode, RuntimeSettings},
        value_objects::sol_amount::Lamports,
    },
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

async fn run_inner() -> Result<(), AppError> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if maybe_handle_service_command(&args)? {
        return Ok(());
    }

    maybe_print_startup_banner();

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

    let network_path = NetworkPathProfile::from_settings(&settings);
    let fpga_feed = FpgaFeedAdapter::new(
        settings.fpga_vendor.as_str().to_owned(),
        settings.fpga_verbose,
    )
    .with_ingress_mode(settings.fpga_ingress_mode)
    .with_direct_device_path(settings.fpga_direct_device_path.as_str().to_owned())
    .with_dma_socket_path(settings.fpga_dma_socket_path.as_str().to_owned());
    let kernel_bypass_stream = SolanaPubsubLogStream::kernel_bypass(
        settings.wss_url.as_str().to_owned(),
        settings.kernel_tcp_bypass_engine,
        settings.kernel_bypass_socket_path.as_str().to_owned(),
    );
    let standard_tcp_stream =
        SolanaPubsubLogStream::standard_tcp(settings.wss_url.as_str().to_owned());

    validate_ingress_readiness(
        &network_path,
        &fpga_feed,
        &kernel_bypass_stream,
        &standard_tcp_stream,
    )?;

    let keypair = Arc::new(load_keypair(&settings.keypair_path).await?);
    let rpc = Arc::new(RpcClient::new(settings.rpc_url.clone()));

    let repository = Arc::new(TomlRuleRepository::new(settings.config_path.clone()));
    let initial_rulebook = load_rulebook(repository.as_ref(), true)
        .await
        .map_err(|source| RulebookLoadError::Read { source })?;

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
        .map_err(|source| WalletBalanceError::Read { source })?;

    let mint_rules = initial_rulebook.mint_log_lines();
    let deployer_rules = initial_rulebook.deployer_log_lines();
    log_runtime_settings(
        &settings,
        &network_path,
        &keypair.pubkey(),
        &balance,
        &mint_rules,
        &deployer_rules,
    );
    maybe_log_fpga_feed(&settings, &fpga_feed);

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
    start_ingress_stream(
        &network_path,
        &fpga_feed,
        &kernel_bypass_stream,
        &standard_tcp_stream,
        events_tx,
    )?;

    let engine = SniperEngine::new(context, events_rx, rulebook_rx, telemetry);
    engine.run().await;

    Ok(())
}

fn log_runtime_settings(
    settings: &RuntimeSettings,
    network_path: &NetworkPathProfile,
    wallet: &solana_sdk::pubkey::Pubkey,
    balance: &str,
    mint_rules: &[String],
    deployer_rules: &[String],
) {
    let mints_string = format_rules(mint_rules);
    let deployers_string = format_rules(deployer_rules);
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
\n\tFPGA_INGRESS_MODE: {}\
\n\tFPGA_DIRECT_DEVICE_PATH: {}\
\n\tFPGA_DMA_SOCKET_PATH: {}\
\n\tTELEMETRY_ENABLED: {}",
        wallet,
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
        settings.fpga_ingress_mode.as_str(),
        settings.fpga_direct_device_path.as_str(),
        settings.fpga_dma_socket_path.as_str(),
        settings.telemetry_enabled,
    );
}

fn maybe_log_fpga_feed(settings: &RuntimeSettings, fpga_feed: &FpgaFeedAdapter) {
    if settings.fpga_enabled {
        log::info!(
            "FPGA_FEED: {} (vendor={}, verbose={})",
            fpga_feed.describe(),
            fpga_feed.vendor(),
            fpga_feed.verbose(),
        );
    }
}

fn start_ingress_stream(
    network_path: &NetworkPathProfile,
    fpga_feed: &FpgaFeedAdapter,
    kernel_bypass_stream: &SolanaPubsubLogStream,
    standard_tcp_stream: &SolanaPubsubLogStream,
    events_tx: mpsc::UnboundedSender<RawLogEvent>,
) -> Result<(), IngressStartupError> {
    match network_path.mode() {
        NetworkStackMode::Fpga => {
            fpga_feed
                .spawn_stream(events_tx)
                .map_err(|source| IngressStartupError::Fpga { source })?;
            log::info!(
                "Ingress path selected: FPGA DMA ring -> strategy events (zero-copy frame parse)"
            );
        }
        NetworkStackMode::KernelBypass => {
            kernel_bypass_stream
                .spawn_stream(events_tx)
                .map_err(|source| IngressStartupError::KernelBypass { source })?;
            log::info!(
                "Ingress path selected: {} -> strategy events",
                kernel_bypass_stream.path_name()
            );
        }
        NetworkStackMode::StandardTcp => {
            standard_tcp_stream
                .spawn_stream(events_tx)
                .map_err(|source| IngressStartupError::StandardTcp { source })?;
            log::info!(
                "Ingress path selected: {} -> strategy events",
                standard_tcp_stream.path_name()
            );
        }
    }

    Ok(())
}

async fn load_keypair(path: &str) -> Result<Keypair, KeypairLoadError> {
    let keypair_path = PathBuf::from(path);
    let mut keypair_file = File::open(path)
        .await
        .map_err(|source| KeypairLoadError::Open {
            path: keypair_path.clone(),
            source,
        })?;

    let mut contents = String::new();
    keypair_file
        .read_to_string(&mut contents)
        .await
        .map_err(|source| KeypairLoadError::Read {
            path: keypair_path.clone(),
            source,
        })?;

    let keypair_bytes = serde_json::from_str::<Vec<u8>>(&contents).map_err(|source| {
        KeypairLoadError::ParseJson {
            path: keypair_path.clone(),
            source,
        }
    })?;

    Keypair::try_from(keypair_bytes.as_slice()).map_err(|source| KeypairLoadError::InvalidBytes {
        path: keypair_path,
        source: Box::new(source),
    })
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

const STARTUP_BANNER: &str = r#"
███████╗██╗      ██████╗ ████████╗███████╗████████╗██████╗ ██╗██╗  ██╗███████╗
██╔════╝██║     ██╔═══██╗╚══██╔══╝██╔════╝╚══██╔══╝██╔══██╗██║██║ ██╔╝██╔════╝
███████╗██║     ██║   ██║   ██║   ███████╗   ██║   ██████╔╝██║█████╔╝ █████╗
╚════██║██║     ██║   ██║   ██║   ╚════██║   ██║   ██╔══██╗██║██╔═██╗ ██╔══╝
███████║███████╗╚██████╔╝   ██║   ███████║   ██║   ██║  ██║██║██║  ██╗███████╗
╚══════╝╚══════╝ ╚═════╝    ╚═╝   ╚══════╝   ╚═╝   ╚═╝  ╚═╝╚═╝╚═╝  ╚═╝╚══════╝"#;

fn maybe_print_startup_banner() {
    if !should_render_local_banner() {
        return;
    }

    println!("{}", render_blue_purple_gradient(STARTUP_BANNER));
}

fn should_render_local_banner() -> bool {
    should_render_local_banner_with(std::io::stdout().is_terminal())
}

const fn should_render_local_banner_with(stdout_is_terminal: bool) -> bool {
    stdout_is_terminal
}

fn render_blue_purple_gradient(text: &str) -> String {
    let visible_count = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    let max_index = visible_count.saturating_sub(1);

    let mut out = String::with_capacity(text.len().saturating_mul(20));
    let mut index = 0usize;

    for character in text.chars() {
        if character.is_whitespace() {
            out.push(character);
            continue;
        }

        let red = gradient_channel(42, 181, index, max_index);
        let green = gradient_channel(106, 64, index, max_index);
        let blue = gradient_channel(255, 255, index, max_index);
        let _write_result = write!(
            out,
            "\u{1b}[38;2;{};{};{}m{}\u{1b}[0m",
            red, green, blue, character
        );
        index = index.saturating_add(1);
    }

    out
}

fn gradient_channel(start: u8, end: u8, index: usize, max_index: usize) -> u8 {
    if max_index == 0 {
        return start;
    }

    let start_u32 = u32::from(start);
    let end_u32 = u32::from(end);
    let span = end_u32.saturating_sub(start_u32);
    let index_u32 = u32::try_from(index).unwrap_or(u32::MAX);
    let max_index_u32 = u32::try_from(max_index).unwrap_or(1);
    let scaled = span
        .saturating_mul(index_u32)
        .checked_div(max_index_u32)
        .unwrap_or(0);
    let value = start_u32.saturating_add(scaled).min(u32::from(u8::MAX));
    u8::try_from(value).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::{gradient_channel, render_blue_purple_gradient, should_render_local_banner_with};

    #[test]
    fn banner_is_disabled_when_stdout_is_not_terminal() {
        assert!(!should_render_local_banner_with(false));
    }

    #[test]
    fn banner_is_enabled_for_local_terminal_runs() {
        assert!(should_render_local_banner_with(true));
    }

    #[test]
    fn gradient_channel_reaches_end_value() {
        let value = gradient_channel(110, 255, 10, 10);
        assert_eq!(value, 255);
    }

    #[test]
    fn gradient_renderer_preserves_whitespace() {
        let rendered = render_blue_purple_gradient("A B");
        assert!(rendered.contains(" "));
    }
}
