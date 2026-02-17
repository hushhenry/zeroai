mod config_tui;
mod doctor;
mod server;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ai-proxy", version, about = "AI model proxy server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the HTTP proxy server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "8787")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Configure providers and models (TUI)
    Config,

    /// Validate credentials for all configured providers (e.g. /v1/models)
    AuthCheck,

    /// Check provider health
    Doctor {
        /// Specific model to check (format: <provider>/<model>)
        #[arg(short, long)]
        model: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ai_proxy=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { port, host } => {
            server::run_server(&host, port).await?;
        }
        Commands::Config => {
            config_tui::run_config_tui().await?;
        }
        Commands::AuthCheck => {
            doctor::run_auth_check().await?;
        }
        Commands::Doctor { model } => {
            doctor::run_doctor(model.as_deref()).await?;
        }
    }

    Ok(())
}
