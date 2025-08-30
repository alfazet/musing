use clap::Parser;
use tokio::{
    signal,
    sync::{
        broadcast,
        mpsc::{self as tokio_chan},
    },
};

use crate::config::{CliOptions, Config};

mod audio;
mod config;
mod constants;
mod database;
mod player;
mod server;
mod state;

mod model;

fn setup_logging(cli_opts: &CliOptions) {
    if cli_opts.log_stderr {
        simple_logging::log_to_stderr(log::LevelFilter::Error);
    } else {
        let default_log_file = dirs::cache_dir()
            .unwrap_or(".".into())
            .join(constants::DEFAULT_LOG_FILE);
        let log_file = cli_opts.log_file.as_deref().unwrap_or(&default_log_file);
        let _ = simple_logging::log_to_file(log_file, log::LevelFilter::Error);
    }
}

#[tokio::main]
async fn main() {
    let cli_opts = CliOptions::parse();
    setup_logging(&cli_opts);
    let Config {
        server_config,
        player_config,
    } = match Config::from_file(cli_opts.config_file.as_deref()).map(|c| c.merge_with_cli(cli_opts))
    {
        Ok(config) => config,
        Err(e) => {
            log::error!("config error ({}), falling back to default", e);
            Config::default()
        }
    };

    let (tx_request, rx_request) = tokio_chan::unbounded_channel();
    // two-way shutdown notification to ensure that state is saved no matter how the program exits
    let (tx_shutdown1, _) = broadcast::channel(1);
    let (tx_shutdown2, mut rx_shutdown2) = broadcast::channel(1);
    let server_task = server::spawn(
        server_config,
        tx_request,
        tx_shutdown1.subscribe(),
        tx_shutdown2.clone(),
    );
    let player_task = player::spawn(
        player_config,
        rx_request,
        tx_shutdown1.subscribe(),
        tx_shutdown2,
    );

    tokio::select! {
        _ = signal::ctrl_c() => (),
        _ = rx_shutdown2.recv() => (),
    };
    let _ = tx_shutdown1.send(());
    let _ = tokio::join!(server_task, player_task);
}
