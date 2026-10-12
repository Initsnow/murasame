pub mod ai;
pub mod app;
pub mod audio;
pub mod config;
pub mod model;
pub mod renderer;

use app::Application;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting Murasame Desktop Pet...");
    let app = Application::new()?;
    app.run()?;
    Ok(())
}
