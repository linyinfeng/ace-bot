use clap::Parser;

use once_cell::sync::Lazy;
use regex::Regex;
use regex::RegexBuilder;
use reqwest::multipart;
use reqwest::multipart::Part;
use reqwest::StatusCode;
use std::path::PathBuf;
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
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
struct Options {
    #[arg(short, long, default_value = "60")]
    pub timeout: usize,
    #[arg(short, long, default_value = "/bin/sh")]
    pub shell: String,
    #[arg(short, long)]
    pub manager_chat_id: Option<i64>,
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

static OPTIONS: Lazy<Options> = Lazy::new(Options::parse);
static USER_COMMAND_PATTERN: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new("^(/user@[a-zA-Z_]+|/user)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static ROOT_COMMAND_PATTERN: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new("^(/root@[a-zA-Z_]+|/root)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static RESET_COMMAND_PATTER: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new("^(/reset@[a-zA-Z_]+|/reset)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

#[derive(Clone, Copy, Debug)]
enum Mode {
    User,
    Root,
}

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
                    if RESET_COMMAND_PATTER.is_match(raw_text) {
                        tokio::spawn(handle_reset(message.clone(), bot.clone(), user.clone()));
                        return Ok(());
                    }

                    let (mode, cleaned) = match USER_COMMAND_PATTERN.captures(raw_text) {
                        Some(c) => (Mode::User, c[2].to_string()),
                        None => match ROOT_COMMAND_PATTERN.captures(raw_text) {
                            Some(c) => (Mode::Root, c[2].to_string()),
                            None => {
                                if message.chat.id.is_user() {
                                    (Mode::User, raw_text.to_string())
                                } else {
                                    log::debug!("ignored update: {:?}", message);
                                    return Ok(());
                                }
                            }
                        },
                    };
                    let bash_command = preprocessing(&cleaned);

                    log::info!("{:?} ({:?}): {}", user, mode, bash_command);
                    tokio::spawn(handle_command(
                        message.clone(),
                        bot,
                        user.clone(),
                        mode,
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

async fn handle_command(message: Message, bot: Bot, user: User, mode: Mode, bash_command: String) {
    if let Err(e) = handle_command_result(message, bot, user, mode, bash_command).await {
        log::warn!("request error: {}", e)
    }
}

async fn handle_reset(message: Message, bot: Bot, user: User) {
    if let Err(e) = handle_reset_result(message, bot, user).await {
        log::warn!("request error: {}", e)
    }
}

async fn handle_command_result(
    message: Message,
    bot: Bot,
    user: User,
    mode: Mode,
    bash_command: String,
) -> ResponseResult<()> {
    match run_command(mode, &bash_command).await {
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

async fn handle_reset_result(message: Message, bot: Bot, user: User) -> ResponseResult<()> {
    match run_reset().await {
        Err(e) => {
            e.report(&message, &bot).await?;
        }
        Ok(output) => {
            let output_message = OutputMessage::format("/reset", &user, output).await;
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

async fn run_command(mode: Mode, text: &str) -> Result<Output, AceError> {
    let mut command = tokio::process::Command::new("systemd-run");
    command.args([
        &format!("--machine={}", OPTIONS.machine),
        "--collect",
        "--quiet",
        "--wait",
        "--pipe",
        "--service-type=oneshot",
        &format!("--property=TimeoutStartSec={}", OPTIONS.timeout),
        "--send-sighup",
    ]);
    match mode {
        Mode::User => {
            command.args([
                &format!("--uid={}", OPTIONS.user_mode_uid),
                &format!("--gid={}", OPTIONS.user_mode_gid),
                &format!("--working-directory={}", OPTIONS.user_home),
            ]);
        }
        Mode::Root => {
            command.args(["--working-directory=/root"]);
        }
    }
    command
        .arg("--")
        .args([&OPTIONS.shell, "--login"])
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

async fn run_reset() -> Result<Output, AceError> {
    File::create(&OPTIONS.reset_indicator).await?;
    let output = tokio::process::Command::new("systemctl")
        .args(["restart", &OPTIONS.machine_unit])
        .output()
        .await?;
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
