use anyhow::Result;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc::UnboundedSender, oneshot},
};

use crate::config::Config;

pub async fn run(config: Config) {
    // construct a Server
    // select! server.run() vs ctrl-c
    // inside of server.run spawn the player task
}
