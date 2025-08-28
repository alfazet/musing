use anyhow::Result;
use clap::Parser;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

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

    /// Additional file extensions that musing should treat as music files (default: empty)
    #[arg(long = "exts")]
    pub additional_exts: Option<Vec<String>>,
}

#[derive(Debug)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug)]
pub struct PlayerConfig {
    pub audio_device: Option<String>,
    pub music_dir: PathBuf,
    pub playlist_dir: PathBuf,
    pub allowed_exts: HashSet<String>,
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
        let music_dir = PathBuf::from(constants::DEFAULT_MUSIC_DIR);
        let playlist_dir = music_dir.join(Path::new(constants::DEFAULT_PLAYLIST_DIR));
        Self {
            audio_device: None,
            music_dir,
            playlist_dir,
            allowed_exts: HashSet::from(constants::DEFAULT_ALLOWED_EXTS.map(|s| s.to_string())),
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
        };
        let allowed_exts: HashSet<_> = cli_opts
            .additional_exts
            .unwrap_or_default()
            .into_iter()
            .collect();
        let allowed_exts: HashSet<_> = allowed_exts
            .union(&self.player_config.allowed_exts)
            .map(|s| s.to_string())
            .collect();

        let player_config = PlayerConfig {
            audio_device: cli_opts.audio_device,
            music_dir: cli_opts.music_dir.unwrap_or(self.player_config.music_dir),
            playlist_dir: cli_opts
                .playlist_dir
                .unwrap_or(self.player_config.playlist_dir),
            allowed_exts,
        };

        Config {
            server_config,
            player_config,
        }
    }
}
