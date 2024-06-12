use clap::Parser;

use once_cell::sync::Lazy;
use regex::Regex;
use regex::RegexBuilder;
use reqwest::multipart;
use reqwest::multipart::Part;
use reqwest::StatusCode;
use std::process::{Output, Stdio};
use teloxide::types::InputFile;
use teloxide::types::InputMedia;
use teloxide::types::InputMediaDocument;
use teloxide::types::{ParseMode, User};
use teloxide::{
    prelude::*,
    requests::ResponseResult,
    types::{MediaKind, MessageKind},
    utils,
};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
pub struct Options {
    #[arg(short, long, default_value = "60")]
    pub timeout: usize,
    #[arg(short, long, default_value = "/bin/sh")]
    pub shell: String,
    #[arg(short, long)]
    pub manager_chat_id: Option<i64>,
    #[arg(short, long)]
    pub working_directory: String,
    #[arg(short, long)]
    pub root_directory: String,
    #[arg(short, long)]
    pub environment: String,
}

static OPTIONS: Lazy<Options> = Lazy::new(Options::parse);
static BOT_COMMAND_PATTERN: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new("^(/bash@[a-zA-Z_]+|/bash)[[:space:]]+(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

#[tokio::main]
async fn main() {
    run().await;
}

async fn run() {
    pretty_env_logger::init();
    log::info!("Starting ace-bot...");
    log::info!("Options = {:#?}", *OPTIONS);
    let bot = Bot::from_env();
    teloxide::repl(bot, handle_update).await;
}

async fn handle_update(message: Message, bot: Bot) -> ResponseResult<()> {
    match &message.kind {
        MessageKind::Common(common_msg) => match &common_msg.media_kind {
            MediaKind::Text(text_media) => match &common_msg.from {
                Some(user) => {
                    let raw_text = &text_media.text;
                    log::debug!("{:?} raw: {}", user, raw_text);
                    let cleaned = match BOT_COMMAND_PATTERN.captures(raw_text) {
                        Some(c) => c[2].to_string(),
                        None => raw_text.to_string(),
                    };
                    let bash_command = preprocessing(&cleaned);

                    log::info!("{:?}: {}", user, bash_command);
                    tokio::spawn(handle_command(
                        message.clone(),
                        bot,
                        user.clone(),
                        bash_command,
                    ));
                }
                _ => log::debug!("ignored update: {:?}", message),
            },
            _ => log::debug!("ignored update: {:?}", message),
        },
        _ => log::debug!("ignored update: {:?}", message),
    }
    Ok(())
}

fn preprocessing(raw: &str) -> String {
    let mut text = raw.replace('—', "--");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

async fn handle_command(message: Message, bot: Bot, user: User, bash_command: String) {
    if let Err(e) = handle_command_result(message, bot, user, bash_command).await {
        log::warn!("request error: {}", e)
    }
}

async fn handle_command_result(
    message: Message,
    bot: Bot,
    user: User,
    bash_command: String,
) -> ResponseResult<()> {
    match run_command(&bash_command).await {
        Err(e) => {
            e.report(&message, &bot).await?;
        }
        Ok(output) => {
            let output_message = OutputMessage::format(&bash_command, &user, output).await;
            let manager_id = match OPTIONS.manager_chat_id {
                Some(id) => ChatId(id),
                None => return Ok(()),
            };
            output_message.send(&bot, message.chat.id).await?;
            if manager_id != message.chat.id {
                output_message.send(&bot, manager_id).await?;
            }
        }
    }
    Ok(())
}

async fn run_command(text: &str) -> Result<Output, AceError> {
    let mut child = tokio::process::Command::new("systemd-run")
        .args([
            "--uid=ace-bot",
            "--gid=ace-bot",
            "--collect",
            "--quiet",
            "--wait",
            "--pipe",
            "--service-type=oneshot",
            &format!("--property=TimeoutStartSec={}", OPTIONS.timeout),
            &format!("--property=RootDirectory={}", OPTIONS.root_directory),
            &format!("--working-directory={}", OPTIONS.working_directory),
            &format!("--property=BindPaths={}", OPTIONS.working_directory),
            &format!("--setenv=PATH={}/bin:/run/current-system/bin", OPTIONS.environment),
            "--slice=acebot.slice",
            "--send-sighup",
            // security
            "--property=NoNewPrivileges=true",
            "--property=RemoveIPC=true",
            "--property=PrivateTmp=true",
            "--property=CapabilityBoundingSet=",
            "--property=PrivateDevices=true",
            "--property=ProtectClock=true",
            "--property=ProtectKernelLogs=true",
            "--property=ProtectKernelModules=true",
            "--property=ProtectControlGroups=true",
            "--property=PrivateMounts=true",
            "--property=SystemCallArchitectures=native",
            "--property=MemoryDenyWriteExecute=true",
            "--property=RestrictNamespaces=true",
            "--property=RestrictSUIDSGID=true",
            "--property=ProtectHostname=true",
            "--property=LockPersonality=true",
            "--property=ProtectKernelTunables=true",
            "--property=RestrictRealtime=true",
            "--property=ProtectSystem=strict",
            "--property=ProtectProc=invisible",
            "--property=ProcSubset=pid",
            "--property=ProtectHome=true",
            "--property=PrivateUsers=true",
            "--property=PrivateTmp=true",
            "--property=SystemCallFilter=@system-service",
            "--property=SystemCallErrorNumber=EPERM",
            // relaxing
            "--property=BindReadOnlyPaths=/dev/log /run/systemd/journal/socket /run/systemd/journal/stdout",
            "--property=BindReadOnlyPaths=/nix",
            "--property=BindReadOnlyPaths=/run/current-system/bin",
        ])
        .arg("--")
        .args([&OPTIONS.shell, "--login"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);
    let output = child.wait_with_output().await?;
    Ok(output)
}

#[derive(Debug)]
pub struct OutputMessage {
    message: String,
    documents: Vec<InputMedia>,
}

impl OutputMessage {
    async fn format(bash_command: &str, user: &User, output: Output) -> OutputMessage {
        let user = user_indicator(user);
        const PART_LIMIT: usize = 1000;
        const FILE_LIMIT: usize = 1024 * 1024; // 1 MiB

        let mut message = String::new();
        let mut documents = Vec::new();
        let client = reqwest::Client::new();

        message.push_str(&format!("{}:\n", utils::markdown::escape(&user)));
        if bash_command.len() < PART_LIMIT {
            message.push_str(&utils::markdown::code_block(bash_command.trim()));
        } else {
            documents.push(InputMedia::Document(InputMediaDocument::new(
                InputFile::memory(Vec::from(bash_command.as_bytes())).file_name("script"),
            )));
        }
        message.push_str(&format!("{}", output.status));
        if !output.stdout.is_empty() {
            message.push_str(&format!("\n{}", utils::markdown::escape("(stdout)")));
            let mut inlined = false;
            if let Ok(s) = String::from_utf8(output.stdout.clone()) {
                if s.len() < PART_LIMIT {
                    inlined = true;
                    message.push_str(&format!("\n{}", utils::markdown::code_block(&s)));
                }
            }
            if !inlined {
                if output.stdout.len() < FILE_LIMIT {
                    message.push_str("\nattached");
                    if let Some(cmd) =
                        pastebin_command(&client, "stdout", output.stdout.clone()).await
                    {
                        message.push_str(&format!("\n{}", utils::markdown::code_block(&cmd)))
                    }
                    documents.push(InputMedia::Document(InputMediaDocument::new(
                        InputFile::memory(output.stdout).file_name("stdout"),
                    )));
                } else {
                    message.push_str("\nfile size limit exceeded");
                }
            }
        }

        if !output.stderr.is_empty() {
            message.push_str(&format!("\n{}", utils::markdown::escape("(stderr)")));
            let mut inlined = false;
            if let Ok(s) = String::from_utf8(output.stderr.clone()) {
                if s.len() < PART_LIMIT {
                    inlined = true;
                    message.push_str(&format!("\n{}", utils::markdown::code_block(&s)));
                }
            }
            if !inlined {
                if output.stderr.len() < FILE_LIMIT {
                    message.push_str("\nattached");
                    if let Some(cmd) =
                        pastebin_command(&client, "stderr", output.stderr.clone()).await
                    {
                        message.push_str(&format!("\n{}", utils::markdown::code_block(&cmd)))
                    }
                    documents.push(InputMedia::Document(InputMediaDocument::new(
                        InputFile::memory(output.stderr).file_name("stderr"),
                    )));
                } else {
                    message.push_str("\nfile size limit exceeded");
                }
            }
        }

        OutputMessage { message, documents }
    }

    async fn send(&self, bot: &Bot, chat_id: ChatId) -> ResponseResult<()> {
        let msg = bot
            .send_message(chat_id, &self.message)
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        if !self.documents.is_empty() {
            bot.send_media_group(chat_id, self.documents.iter().cloned())
                .reply_to_message_id(msg.id)
                .await?;
        }
        Ok(())
    }
}

async fn pastebin_command(
    client: &reqwest::Client,
    file_name: &str,
    data: Vec<u8>,
) -> Option<String> {
    let form = multipart::Form::new().part("c", Part::bytes(data).file_name(file_name.to_string()));
    let result = client
        .post("https://pb.li7g.com")
        .multipart(form)
        .send()
        .await;
    match result {
        Ok(response) => {
            if response.status() == StatusCode::OK {
                match response.text().await {
                    Ok(url) => Some(format!("curl {}", url.trim())),
                    Err(e) => {
                        log::warn!("pastebin body error: {e:#?}");
                        None
                    }
                }
            } else {
                log::warn!("pastebin invalid response: {response:#?}");
                log::warn!("  body: {:?}", response.text().await);
                None
            }
        }
        Err(e) => {
            log::warn!("pastebin error: {e:#?}");
            None
        }
    }
}

fn user_indicator(user: &User) -> String {
    if let Some(s) = &user.username {
        return format!("@{}", s);
    }
    user.first_name.to_string()
}

#[derive(thiserror::Error, Debug)]
pub enum AceError {
    #[error("timeout reached")]
    Timeout,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl AceError {
    pub async fn report(&self, msg: &Message, bot: &Bot) -> Result<(), teloxide::RequestError> {
        log::warn!("report error to chat {}: {:?}", msg.chat.id, self);
        bot.send_message(msg.chat.id, format!("{}", self))
            .reply_to_message_id(msg.id)
            .await?;
        Ok(())
    }
}
