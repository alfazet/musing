use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};

use crate::constants;

#[derive(Parser, Debug)]
#[command(version, about, author, long_about = None)]
pub struct CliOptions {
    /// Path to the music directory, all paths will be processed as relative to it.
    #[arg(short = 'm', long = "music")]
    pub music_dir: Option<PathBuf>,

    /// Port on which rustmpd will listen for clients.
    #[arg(short = 'p', long = "port")]
    pub port: Option<u16>,

    /// Audio device to be used for playback
    #[arg(short = 'd', long = "device")]
    pub audio_device_name: Option<String>,

    /// Path to the rustmpd.toml config file.
    #[arg(short = 'c', long = "config")]
    pub config_file: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug)]
pub struct PlayerConfig {
    pub music_dir: PathBuf,
    pub audio_device_name: String,
}

#[derive(Debug, Default)]
pub struct Config {
    pub server_config: ServerConfig,
    pub player_config: PlayerConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            port: constants::DEFAULT_PORT,
        }
    }
}

impl ServerConfig {
    // pub fn new() -> Self {
    //     ServerConfig::default()
    // }

    // pub fn from_toml_pairs() -> Result<Self> {
    //     // TODO: this
    //     Ok(ServerConfig::default())
    // }
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            music_dir: constants::DEFAULT_MUSIC_DIR.into(),
            audio_device_name: constants::DEFAULT_AUDIO_DEVICE.to_string(),
        }
    }
}

impl PlayerConfig {
    // fn new() -> Self {
    //     PlayerConfig::default()
    // }

    // pub fn from_toml_pairs() -> Result<Self> {
    //     // TODO: this
    //     Ok(PlayerConfig::default())
    // }
}

impl Config {
    // pub fn new() -> Self {
    //     Config::default()
    // }

    pub fn from_file(_path: Option<&Path>) -> Result<Self> {
        // let default_path = dirs::config_dir()
        //     .unwrap_or(dirs::home_dir().unwrap())
        //     .join(constants::DEFAULT_CONFIG_FILE);
        // let path = path.unwrap_or(&default_path);
        // if path doesn't exist return the default
        // read the toml file
        // server_config = ..from_toml_pairs()?
        // player_config = ditto
        // Ok(Config { server_config, player_config })
        Ok(Config::default())
    }

    pub fn merge_with_cli(self, cli_opts: CliOptions) -> Self {
        let server_config = ServerConfig {
            port: cli_opts.port.unwrap_or(self.server_config.port),
        };
        let player_config = PlayerConfig {
            music_dir: cli_opts.music_dir.unwrap_or(self.player_config.music_dir),
            audio_device_name: cli_opts
                .audio_device_name
                .unwrap_or(self.player_config.audio_device_name),
        };

        Config {
            server_config,
            player_config,
        }
    }
}
