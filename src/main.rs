use once_cell::sync::Lazy;
use regex::Regex;
use regex::RegexBuilder;
use std::{
    process::{Output, Stdio},
    time::Duration,
};
use teloxide::types::InputFile;
use teloxide::types::{ParseMode, Recipient, User};
use teloxide::{
    prelude::*,
    requests::ResponseResult,
    types::{MediaKind, MessageKind},
    utils,
};
use tokio::io::AsyncWriteExt;
use tokio::select;
use tokio::time::sleep;

static MANAGER_CHAT_ID: Lazy<Option<i64>> = Lazy::new(|| match std::env::var("MANAGER_CHAT_ID") {
    Ok(s) => s.parse().ok(),
    Err(_) => None,
});
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
    let bot = Bot::from_env();
    teloxide::repl(bot, handle_update).await;
}

async fn handle_update(message: Message, bot: Bot) -> ResponseResult<()> {
    match &message.kind {
        MessageKind::Common(common_msg) => match &common_msg.media_kind {
            MediaKind::Text(text_media) => match &common_msg.from {
                Some(user) => {
                    let raw_text = &text_media.text;
                    log::info!("{:?} raw: {}", user, raw_text);
                    let cleaned = match BOT_COMMAND_PATTERN.captures(raw_text) {
                        Some(c) => c[2].to_string(),
                        None => raw_text.to_string(),
                    };
                    let bash_command = preprocessing(&cleaned);

                    log::info!("{:?}: {}", user, bash_command);
                    log_for_manager(&bot, user, &bash_command).await?;
                    match handle_command(&bash_command).await {
                        Err(e) => {
                            e.report(&message, &bot).await?;
                        }
                        Ok(output) => {
                            log::info!("command '{:?}': output: {:?}", bash_command, output);
                            let mut output_message = String::new();
                            output_message
                                .push_str(&utils::markdown::code_block(bash_command.trim()));
                            output_message.push_str(&format!("{}", output.status));
                            if !output.stdout.is_empty() {
                                output_message.push_str(&format!(
                                    "\n{}\n{}",
                                    utils::markdown::escape("(stdout)"),
                                    utils::markdown::code_block(&String::from_utf8_lossy(
                                        &output.stdout
                                    ))
                                ));
                            }
                            if !output.stderr.is_empty() {
                                output_message.push_str(&format!(
                                    "\n{}\n{}",
                                    utils::markdown::escape("(stderr)"),
                                    utils::markdown::code_block(&String::from_utf8_lossy(
                                        &output.stderr
                                    ))
                                ));
                            }
                            if output_message.len() >= 4000 {
                                let document = InputFile::memory(output_message);
                                bot.send_document(message.chat.id, document)
                                    .reply_to_message_id(message.id)
                                    .await?;
                            } else {
                                bot.send_message(message.chat.id, output_message)
                                    .reply_to_message_id(message.id)
                                    .parse_mode(ParseMode::MarkdownV2)
                                    .await?;
                            }
                        }
                    }
                }
                _ => log::info!("ignored update: {:?}", message),
            },
            _ => log::info!("ignored update: {:?}", message),
        },
        _ => log::info!("ignored update: {:?}", message),
    }
    Ok(())
}

async fn log_for_manager(bot: &Bot, user: &User, text: &str) -> ResponseResult<()> {
    let manager_id = match *MANAGER_CHAT_ID {
        Some(id) => id,
        None => return Ok(()),
    };
    let last_name = match &user.last_name {
        Some(l) => l.to_string(),
        None => String::new(),
    };
    bot.send_message(
        Recipient::Id(ChatId(manager_id)),
        format!(
            "{} {}:\n{}",
            utils::markdown::escape(&user.first_name),
            utils::markdown::escape(&last_name),
            utils::markdown::code_block(text)
        ),
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;
    Ok(())
}

fn preprocessing(raw: &str) -> String {
    let mut text = raw.replace('—', "--");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

async fn handle_command(text: &str) -> Result<Output, AceError> {
    let timeout = sleep(Duration::from_secs(60));

    let mut child = tokio::process::Command::new("bash")
        .env_remove("TELOXIDE_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);

    select! {
        _ = child.wait() => {
            let output = child.wait_with_output().await?;
            Ok(output)
        }
        _ = timeout => {
            child.kill().await.expect("failed to kill, just abort");
            Err(AceError::Timeout)
        }
    }
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
