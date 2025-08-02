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

    // use crate::database::*;
    // use crate::model::filter::*;
    // use crate::parsers::filter::*;
    // use std::path::PathBuf;
    //
    // let db = Database::from_dir(
    //     &PathBuf::from("/home/antek/Music/tracks"),
    //     vec!["mp3".into()],
    // );
    // let filter_expr = FilterExpr::try_from(String::from(r#"albumartist=="ILLENIUM""#)).unwrap();
    // let resp = db.select(filter_expr);
    // println!("{:?}", resp.into_json_string());
    //
    // let ids = vec![
    //     6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 19, 24, 331, 332, 333, 334, 335, 336, 337, 338, 339,
    //     340, 341, 342, 343, 344, 345, 346, 347, 348, 349, 350, 351, 352, 353, 354, 355, 356, 357,
    //     358, 359, 360, 361, 362, 363, 364, 365, 366, 367, 368, 369, 370, 371, 372, 373, 374, 375,
    //     376, 377, 378, 379, 380, 381, 382, 383, 384, 385, 386, 387, 388, 389, 390, 391, 392, 393,
    //     394, 395, 396, 397, 398, 399, 400,
    // ];
    // let resp = db.metadata((ids, vec!["tracktitle".into(), "album".into()]));
    // println!("{}", resp.into_json_string().unwrap());
}
