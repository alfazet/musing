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
    // two-way shutdown notification to ensure that state is saved before the program exits
    let (tx_shutdown1, rx_shutdown1) = broadcast::channel(1);
    let (tx_shutdown2, mut rx_shutdown2) = broadcast::channel(1);
    let rx_shutdown1_ = tx_shutdown1.subscribe();
    let tx_shutdown2_ = tx_shutdown2.clone();
    let server_task = tokio::spawn(async move {
        let res = server::run(server_config, tx_request, rx_shutdown1).await;
        if let Err(e) = res {
            log::error!("fatal error ({})", e);
        }
        let _ = tx_shutdown2.send(());
    });
    let player_task = tokio::spawn(async move {
        let res = player::run(player_config, rx_request, rx_shutdown1_).await;
        if let Err(e) = res {
            log::error!("fatal error ({})", e);
        }
        let _ = tx_shutdown2_.send(());
    });

    tokio::select! {
        _ = signal::ctrl_c() => (),
        _ = rx_shutdown2.recv() => (),
    };
    // make sure that state is saved (in case it's the server that crashed)
    let _ = tx_shutdown1.send(());
    let _ = tokio::join!(server_task, player_task);
}
