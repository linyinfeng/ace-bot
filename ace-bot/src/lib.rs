use clap::Parser;

pub mod pastebin;

use std::fmt;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[derive(Debug)]
pub struct AceBot {
    options: Options,
}

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
pub struct Options {
    #[arg(short, long, default_value = "60")]
    pub timeout: usize,
    #[arg(short, long, default_value = "/bin/sh")]
    pub shell: String,
    #[arg(long)]
    pub user_mode_uid: String,
    #[arg(long)]
    pub user_mode_gid: String,
    #[arg(long)]
    pub user_home: String,
    #[arg(long)]
    pub machine: String,
    #[arg(long)]
    pub reset_indicator: PathBuf,
    #[arg(long)]
    pub machine_unit: String,
}

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    NonRoot,
    Root,
}

#[derive(thiserror::Error, Debug)]
pub enum AceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl AceBot {
    pub fn new(options: Options) -> Self {
        Self { options }
    }

    pub async fn run(&self, mode: Mode, text: &str) -> Result<Output, AceError> {
        let mut command = tokio::process::Command::new("systemd-run");
        command.args([
            &format!("--machine={}", self.options.machine),
            "--collect",
            "--quiet",
            "--wait",
            "--pipe",
            "--service-type=oneshot",
            &format!("--property=TimeoutStartSec={}", self.options.timeout),
            "--send-sighup",
        ]);
        match mode {
            Mode::NonRoot => {
                command.args([
                    &format!("--uid={}", self.options.user_mode_uid),
                    &format!("--gid={}", self.options.user_mode_gid),
                    &format!("--working-directory={}", self.options.user_home),
                ]);
            }
            Mode::Root => {
                command.args(["--working-directory=/root"]);
            }
        }
        command
            .arg("--")
            .args([&self.options.shell, "--login"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(text.as_bytes()).await?;
        drop(stdin);
        let output = child.wait_with_output().await?;
        Ok(output)
    }

    pub async fn reset(&self) -> Result<Output, AceError> {
        File::create(&self.options.reset_indicator).await?;
        let output = tokio::process::Command::new("systemctl")
            .args(["restart", &self.options.machine_unit])
            .output()
            .await?;
        Ok(output)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mode::Root => write!(f, "root"),
            Mode::NonRoot => write!(f, "non-root"),
        }
    }
}
