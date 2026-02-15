use clap::Parser;
use users::{Group, User, get_group_by_name, get_user_by_name};

pub mod pastebin;

use mktemp::Temp;
use std::os::unix::fs::chown;
use std::path::{Path, PathBuf, StripPrefixError};
use std::process::{Output, Stdio};
use std::{fmt, io};
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
    Xelatex,
    Typst,
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
            Mode::Xelatex => self.run_xelatex(text).await,
            Mode::Typst => self.run_typst(text).await,
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
            _ => return Err(AceError::InvalidMode(mode)),
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

    pub async fn run_in_temp_dir<Fn, F>(&self, task: Fn) -> Result<Output, AceError>
    where
        Fn: FnOnce(Temp, PathBuf) -> F,
        F: Future<Output = Result<Output, AceError>>,
    {
        let host_dir = self.options.user_host_home.join(".ace-bot").join("tasks");
        let guest_dir = self.options.user_guest_home.join(".ace-bot").join("tasks");
        create_dir_all(&host_dir).await?;
        let host_temp = Temp::new_dir_in(&host_dir)?;
        let guest_temp = guest_dir.join(
            host_temp
                .strip_prefix(&host_dir)
                .map_err(AceError::CanNotStripTempFilePath)?,
        );
        self.ensure_owner(self.options.user_host_home.join(".ace-bot"))?;
        self.ensure_owner(&host_dir)?;
        self.ensure_owner(&host_temp)?;
        task(host_temp, guest_temp).await
    }

    pub async fn run_nix(&self, expr: &str) -> Result<Output, AceError> {
        self.run_in_temp_dir(async |host_temp, guest_temp| {
            let (mut file, _host_path, guest_path) = self
                .create_file(&host_temp, &guest_temp, "expr.nix")
                .await?;
            let content = format!("let pkgs = import <nixpkgs> {{ }}; in {expr}");
            file.write_all(content.as_bytes()).await?; // utf-8
            file.flush().await?;
            let eval_command = format!("nix eval --file {}", guest_path.display());
            self.run_bash(Mode::NonRoot, &eval_command).await
        })
        .await
    }

    pub async fn run_xelatex(&self, expr: &str) -> Result<Output, AceError> {
        self.run_in_temp_dir(async |host_temp, guest_temp| {
            let (mut file, _host_path, _guest_path) = self
                .create_file(&host_temp, &guest_temp, "main.tex")
                .await?;
            let content = format!(
                r#"\documentclass[dvisvgm, border=5mm]{{standalone}}

\special{{background White}}

\begin{{document}}

{expr}

\end{{document}}
"#
            );
            file.write_all(content.as_bytes()).await?; // utf-8
            file.flush().await?;
            let eval_command = format!(
                r#"cd {}
echo "===== main.tex =====" >&2
cat main.tex >&2
echo "===== xelatex --no-pef main.tex =====" >&2
xelatex --no-pdf main.tex >&2
echo "===== dvisvgm --no-fonts --bbox=papersize main.xdv =====" >&2
dvisvgm --no-fonts --bbox=papersize main.xdv >&2
cat main.svg
"#,
                guest_temp.display()
            );
            self.run_bash(Mode::NonRoot, &eval_command).await
        })
        .await
    }

    pub async fn run_typst(&self, expr: &str) -> Result<Output, AceError> {
        self.run_in_temp_dir(async |host_temp, guest_temp| {
            let (mut file, _host_path, _guest_path) = self
                .create_file(&host_temp, &guest_temp, "main.typ")
                .await?;
            let content = format!(
                r#"#set page(
  width: auto,
  height: auto,
  margin: 5mm,
)
{expr}
"#
            );
            file.write_all(content.as_bytes()).await?; // utf-8
            file.flush().await?;
            let eval_command = format!(
                r#"cd {}
echo "===== main.typ =====" >&2
cat main.typ >&2
echo "===== typst compile --format=svg main.typ =====" >&2
typst compile --format=svg main.typ >&2
cat main.svg
"#,
                guest_temp.display()
            );
            self.run_bash(Mode::NonRoot, &eval_command).await
        })
        .await
    }

    pub fn ensure_owner<P: AsRef<Path>>(&self, path: P) -> Result<(), io::Error> {
        let p = path.as_ref();
        chown(
            p,
            Some(self.user_mode_user.uid()),
            Some(self.user_mode_group.gid()),
        )
    }

    pub async fn create_file(
        &self,
        host_temp: &Temp,
        guest_temp: &Path,
        name: &str,
    ) -> Result<(File, PathBuf, PathBuf), io::Error> {
        let host_path = host_temp.join(name);
        let guest_path = guest_temp.join(name);
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&host_path)
            .await?;
        self.ensure_owner(&host_path)?;
        Ok((file, host_path, guest_path))
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
            Mode::Xelatex => write!(f, "xelatex"),
            Mode::Typst => write!(f, "typst"),
        }
    }
}
