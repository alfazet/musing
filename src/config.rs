use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};

use crate::constants;

#[derive(Parser, Debug)]
#[command(version, about, author, long_about = None)]
pub struct CliOptions {
    /// Port on which musing will listen for clients.
    #[arg(short = 'p', long = "port")]
    pub port: Option<u16>,

    /// Audio device to use as the output.
    #[arg(short = 'd', long = "device")]
    pub audio_device: Option<String>,

    /// Path to the directory containing the music files.
    #[arg(short = 'm', long = "music")]
    pub music_dir: Option<PathBuf>,

    /// Path to the musing.toml config file.
    #[arg(short = 'c', long = "config")]
    pub config_file: Option<PathBuf>,

    /// Path to the log file.
    #[arg(short = 'l', long = "log")]
    pub log_file: Option<PathBuf>,

    /// Print logs to stderr.
    #[arg(long = "stderr")]
    pub log_stderr: bool,
}

#[derive(Debug)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug)]
pub struct PlayerConfig {
    pub audio_device: Option<String>,
    pub music_dir: PathBuf,
    pub allowed_exts: Vec<String>,
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

impl ServerConfig {}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            audio_device: None,
            music_dir: constants::DEFAULT_MUSIC_DIR.into(),
            allowed_exts: constants::DEFAULT_ALLOWED_EXTS
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl PlayerConfig {}

impl Config {
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
            ..self.server_config
        };
        let player_config = PlayerConfig {
            audio_device: cli_opts.audio_device,
            music_dir: cli_opts.music_dir.unwrap_or(self.player_config.music_dir),
            ..self.player_config
        };

        Config {
            server_config,
            player_config,
        }
    }
}
