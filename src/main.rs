use anyhow::Result;
use clap::Parser;

use crate::config::{CliOptions, Config};

mod config;
mod constants;
mod database;
mod error;
mod player;
mod server;
mod utils;

mod model;
mod parsers;

async fn run(cli_options: CliOptions) -> Result<()> {
    let config = Config::from_file(cli_options.config_file.as_deref())?.merge_with_cli(cli_options);
    let Config {
        server_config,
        player_config,
    } = config;

    Ok(())

    // let (tx, player_task) = player::run(player_config)?;
    // let server_task =
    // let res = tokio::select! {
    //     res = server::run(server_config, tx) => res,
    //     _ = tokio::signal::ctrl_c() => Ok(()),
    // };
    // let _ = player_task.await;

    // res
}

#[tokio::main]
async fn main() {
    let _ = simple_logging::log_to_file(
        dirs::cache_dir()
            .unwrap_or(dirs::home_dir().unwrap())
            .join(constants::DEFAULT_LOG_FILE),
        log::LevelFilter::max(),
    );
    let cli_options = CliOptions::parse();
    if let Err(e) = run(cli_options).await {
        log::error!("{}", e);
    }

    use crate::database::*;
    use crate::model::filter::*;
    use crate::parsers::filter::*;
    use std::path::PathBuf;

    let db = Database::from_dir(
        &PathBuf::from("/home/antek/Music/tracks"),
        vec!["mp3".into()],
    );
}
