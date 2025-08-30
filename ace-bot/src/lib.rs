use clap::Parser;
use users::{Group, User, get_group_by_name, get_user_by_name};

pub mod pastebin;

use mktemp::Temp;
use std::fmt;
use std::os::unix::fs::chown;
use std::path::{Path, PathBuf, StripPrefixError};
use std::process::{Output, Stdio};
use tokio::fs::{File, OpenOptions, create_dir_all};
use tokio::io::AsyncWriteExt;

#[derive(Debug)]
pub struct AceBot {
    options: Options,
    user_mode_user: User,
    user_mode_group: Group,
}

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
pub struct Options {
    #[arg(short, long, default_value = "60")]
    pub timeout: usize,
    #[arg(short, long, default_value = "/bin/sh")]
    pub shell: String,
    #[arg(long)]
    pub user_mode_user: String,
    #[arg(long)]
    pub user_mode_group: String,
    #[arg(long)]
    pub user_guest_home: PathBuf,
    #[arg(long)]
    pub user_host_home: PathBuf,
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
    Nix,
}

#[derive(thiserror::Error, Debug)]
pub enum AceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid mode: {0}")]
    InvalidMode(Mode),
    #[error("can not strip temporary file path: {0}")]
    CanNotStripTempFilePath(StripPrefixError),
    #[error("missing user: {0}")]
    MissingUser(String),
    #[error("missing group: {0}")]
    MissingGroup(String),
}

impl AceBot {
    pub fn new(options: Options) -> Result<Self, AceError> {
        let user = get_user_by_name(&options.user_mode_user)
            .ok_or_else(|| AceError::MissingUser(options.user_mode_user.clone()))?;
        let group = get_group_by_name(&options.user_mode_group)
            .ok_or_else(|| AceError::MissingGroup(options.user_mode_user.clone()))?;
        Ok(Self {
            options,
            user_mode_user: user,
            user_mode_group: group,
        })
    }

    pub async fn run(&self, mode: Mode, text: &str) -> Result<Output, AceError> {
        match mode {
            Mode::NonRoot | Mode::Root => self.run_bash(mode, text).await,
            Mode::Nix => self.run_nix(text).await,
        }
    }

    pub async fn run_bash(&self, mode: Mode, text: &str) -> Result<Output, AceError> {
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
                    // systemd-run accepts user/group names
                    &format!("--uid={}", self.options.user_mode_user),
                    &format!("--gid={}", self.options.user_mode_group),
                ]);
                command.arg("--working-directory");
                command.arg(&self.options.user_guest_home);
            }
            Mode::Root => {
                command.args(["--working-directory=/root"]);
            }
            Mode::Nix => return Err(AceError::InvalidMode(mode)),
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
        stdin.flush().await?;
        drop(stdin);
        let output = child.wait_with_output().await?;
        Ok(output)
    }

    pub async fn run_nix(&self, expr: &str) -> Result<Output, AceError> {
        // prepare file
        let host_dir = self.options.user_host_home.join(".ace-bot").join("nix");
        let guest_dir = self.options.user_guest_home.join(".ace-bot").join("nix");
        create_dir_all(&host_dir).await?;
        let temp = Temp::new_file_in(&host_dir)?;
        let mut file = OpenOptions::new().write(true).open(&temp).await?;
        let content = format!("let pkgs = import <nixpkgs> {{ }}; in {expr}");
        file.write_all(content.as_bytes()).await?; // utf-8
        file.flush().await?;

        // ensure permissions
        let ensure_owner = |p: &Path| {
            chown(
                p,
                Some(self.user_mode_user.uid()),
                Some(self.user_mode_group.gid()),
            )
        };
        ensure_owner(&self.options.user_host_home.join(".ace-bot"))?;
        ensure_owner(&host_dir)?;
        ensure_owner(&temp)?;

        // evaluate
        let relative_path = temp
            .strip_prefix(&host_dir)
            .map_err(AceError::CanNotStripTempFilePath)?;
        let guest_path = guest_dir.join(relative_path);
        let eval_command = format!("nix eval --file {}", guest_path.display());
        let output = self.run_bash(Mode::NonRoot, &eval_command).await?;
        drop(temp); // temp finally drops here or on error
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
            Mode::Nix => write!(f, "nix"),
        }
    }
}
