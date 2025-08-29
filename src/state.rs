use anyhow::Result;
use bincode::{self, Decode, Encode};
use std::{fs::File, path::Path};

use crate::model::{
    decoder::{Speed, Volume},
    queue::Queue,
};

#[derive(Decode, Encode)]
pub struct AudioState {
    pub volume: Volume,
    pub speed: Speed,
    pub gapless: bool,
}

#[derive(Decode, Encode)]
pub struct PlayerState {
    pub queue: Queue,
}

#[derive(Decode, Encode)]
pub struct State {
    pub audio_state: AudioState,
    pub player_state: PlayerState,
}

impl State {
    pub fn try_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let mut content = File::open(path.as_ref())?;
        Ok(bincode::decode_from_std_read(
            &mut content,
            bincode::config::standard(),
        )?)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut file = File::create(path.as_ref())?;
        bincode::encode_into_std_write(self, &mut file, bincode::config::standard())?;

        Ok(())
    }
}
