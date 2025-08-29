use anyhow::{Result, anyhow};
use clap::Parser;
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml::{Table, Value};

use crate::constants;

#[derive(Parser, Debug)]
#[command(version, about, author, long_about = None)]
pub struct CliOptions {
    /// Audio device to use as the output (default: the system's default).
    #[arg(short = 'd', long = "device")]
    pub audio_device: Option<String>,

    /// Path to the directory containing the music files (default: the process' CWD).
    #[arg(short = 'm', long = "music")]
    pub music_dir: Option<PathBuf>,

    /// Path to the directory containing the playlist (.m3u or .m3u8) files (default: <music_dir>/playlists).
    #[arg(short = 'p', long = "playlists")]
    pub playlist_dir: Option<PathBuf>,

    /// Path to the musing.toml config file (default: <config_dir>/musing/musing.toml).
    #[arg(short = 'c', long = "config")]
    pub config_file: Option<PathBuf>,

    /// Path to the log file (default: <cache_dir>/musing.log).
    #[arg(long = "log")]
    pub log_file: Option<PathBuf>,

    /// Port on which musing will listen for clients (default: 2137).
    #[arg(long = "port")]
    pub port: Option<u16>,

    /// Print logs to stderr (default: false).
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
    pub playlist_dir: Option<PathBuf>,
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
    pub fn try_new(content: impl AsRef<str>) -> Result<Self> {
        let mut config = Self::default();
        let table = content.as_ref().parse::<Table>()?;
        for (key, val) in table {
            if let ("port", Value::Integer(port)) = (key.as_str(), val) {
                config.port = port as u16;
            }
        }

        Ok(config)
    }
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            audio_device: None,
            music_dir: PathBuf::from(constants::DEFAULT_MUSIC_DIR),
            playlist_dir: None,
        }
    }
}

impl PlayerConfig {
    pub fn try_new(content: impl AsRef<str>) -> Result<Self> {
        let mut config = Self::default();
        let table = content.as_ref().parse::<Table>()?;
        for (key, val) in table {
            match (key.as_str(), val) {
                ("audio_device", Value::String(audio_device)) => {
                    config.audio_device = Some(audio_device);
                }
                ("music_dir", Value::String(music_dir)) => {
                    config.music_dir = music_dir.into();
                }
                ("playlist_dir", Value::String(playlist_dir)) => {
                    config.playlist_dir = Some(playlist_dir.into());
                }
                _ => (),
            }
        }

        Ok(config)
    }
}

impl Config {
    pub fn from_file(path: Option<&Path>) -> Result<Self> {
        let default_path = dirs::config_dir()
            .ok_or(anyhow!("no config dir found on the system"))?
            .join(constants::DEFAULT_CONFIG_DIR)
            .join(constants::DEFAULT_CONFIG_FILE);
        let path = path.unwrap_or(&default_path);
        let content = fs::read_to_string(path)?;
        let server_config = ServerConfig::try_new(&content)?;
        let player_config = PlayerConfig::try_new(&content)?;

        Ok(Self {
            server_config,
            player_config,
        })
    }

    pub fn merge_with_cli(self, cli_opts: CliOptions) -> Self {
        let server_config = ServerConfig {
            port: cli_opts.port.unwrap_or(self.server_config.port),
        };
        let player_config = PlayerConfig {
            audio_device: cli_opts.audio_device.or(self.player_config.audio_device),
            music_dir: cli_opts.music_dir.unwrap_or(self.player_config.music_dir),
            playlist_dir: cli_opts.playlist_dir.or(self.player_config.playlist_dir),
        };

        Config {
            server_config,
            player_config,
        }
    }
}
