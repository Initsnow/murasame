use murasame::app::Application;
use murasame::paths;

fn main() -> anyhow::Result<()> {
    // Ensure working directory is set to application root if running from elsewhere
    let root = paths::app_root_dir();
    let _ = std::env::set_current_dir(root);

    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}\n");
        let log_path = paths::crash_log_path();
        let _ = std::fs::write(log_path, msg);
    }));

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting Murasame Desktop Pet...");
    tracing::info!("App root: {}", paths::app_root_dir().display());
    tracing::info!("Data dir: {}", paths::data_dir().display());

    let res = (|| -> anyhow::Result<()> {
        let app = Application::new()?;
        app.run()?;
        Ok(())
    })();
    if let Err(ref e) = res {
        let log_path = paths::crash_log_path();
        let _ = std::fs::write(log_path, format!("APP ERROR: {e:?}\n"));
    }
    res
}
