use clap::Parser;

use crate::config::{CliOptions, Config};

mod audio;
mod config;
mod constants;
mod database;
mod error;
mod player;
mod server;
mod utils;

mod model;
mod parsers;

fn setup_logging(cli_opts: &CliOptions) {
    if cli_opts.log_stderr {
        simple_logging::log_to_stderr(log::LevelFilter::Warn);
    } else {
        let default_log_file = dirs::cache_dir()
            .unwrap_or(dirs::home_dir().unwrap())
            .join(constants::DEFAULT_LOG_FILE);
        let log_file = cli_opts.log_file.as_deref().unwrap_or(&default_log_file);
        let _ = simple_logging::log_to_file(log_file, log::LevelFilter::Warn);
    }
}

#[tokio::main]
async fn main() {
    let cli_opts = CliOptions::parse();
    setup_logging(&cli_opts);
    let config = match Config::from_file(cli_opts.config_file.as_deref())
        .map(|c| c.merge_with_cli(cli_opts))
    {
        Ok(config) => config,
        Err(e) => {
            log::error!("{}", e);
            return;
        }
    };

    let res = tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        res = server::run(config) => res,
    };
    if let Err(e) = res {
        log::error!("{}", e);
    }
}
