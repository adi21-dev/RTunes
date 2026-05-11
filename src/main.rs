//! RTunes — terminal music player.

use clap::Parser;

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show,
        );
        original(info);
    }));
}

fn init_tracing(log_level_override: Option<&str>) -> tracing_appender::non_blocking::WorkerGuard {
    let dir = rtunes::config::log_dir();
    let appender = tracing_appender::rolling::daily(&dir, "rtunes.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    let filter = if let Some(level) = log_level_override {
        tracing_subscriber::EnvFilter::new(level)
    } else {
        tracing_subscriber::EnvFilter::try_from_env("RTUNES_LOG_LEVEL")
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    guard
}

fn main() -> anyhow::Result<()> {
    install_panic_hook();

    let cli = rtunes::cli::Cli::parse();
    let cfg_path = rtunes::config::resolved_config_path(cli.config.as_deref());
    rtunes::config::ensure_dirs_for(&cfg_path)?;

    let _log_guard = init_tracing(cli.log_level.as_deref());

    let mut rtunes_cfg =
        rtunes::config::load_or_create(&cfg_path).map_err(|e| anyhow::anyhow!(e))?;

    tracing::info!(path = %cfg_path.display(), "config loaded");
    rtunes::cli::dispatch(&cli, &cfg_path, &mut rtunes_cfg)
}
