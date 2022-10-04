use once_cell::sync::Lazy;
use std::{
    process::{Output, Stdio},
    time::Duration,
};
use teloxide::types::{ParseMode, Recipient, User};
use teloxide::{
    prelude::*,
    requests::ResponseResult,
    types::{MediaKind, MessageKind},
    utils,
};
use tokio::select;
use tokio::time::sleep;
use tokio::{io::AsyncWriteExt, process::Command};

static MANAGER_CHAT_ID: Lazy<i64> = Lazy::new(|| {
    std::env::var("MANAGER_CHAT_ID")
        .expect("missing MANAGER_CHAT_ID")
        .parse()
        .expect("invalid MANAGER_CHAT_ID")
});

#[tokio::main]
async fn main() {
    run().await;
}

async fn run() {
    pretty_env_logger::init();
    log::info!("Starting ace-bot...");
    let bot = Bot::from_env().auto_send();
    teloxide::repl(bot, handle_update).await;
}

async fn handle_update(message: Message, bot: AutoSend<Bot>) -> ResponseResult<()> {
    match &message.kind {
        MessageKind::Common(common_msg) => match &common_msg.media_kind {
            MediaKind::Text(text_media) => match &common_msg.from {
                Some(user) => {
                    log::info!("handle message: {:?}", message);
                    let raw_text = &text_media.text;
                    let text = preprocessing(raw_text);
                    log::info!("{:?}: {}", user, text);
                    log_for_manager(&bot, &user, &text).await?;
                    match handle_command(&text).await {
                        Err(e) => {
                            e.report(&message, &bot).await?;
                        }
                        Ok(output) => {
                            log::info!("command '{:?}': output: {:?}", text, output);
                            let mut output_message = String::new();
                            output_message.push_str(&format!("{}", output.status));
                            if !output.stdout.is_empty() {
                                output_message.push_str(&format!(
                                    "\n{}\n{}",
                                    utils::markdown::escape("(stdout)"),
                                    utils::markdown::code_block(&String::from_utf8_lossy(&output.stdout))
                                ));
                            }
                            if !output.stderr.is_empty() {
                                output_message.push_str(&format!(
                                    "\n{}\n{}",
                                    utils::markdown::escape("(stderr)"),
                                    utils::markdown::code_block(&String::from_utf8_lossy(&output.stderr))
                                ));
                            }
                            if output_message.len() >= 4000 {
                                bot.send_message(message.chat.id, "error: output message is too long")
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

async fn log_for_manager(bot: &AutoSend<Bot>, user: &User, text: &str) -> ResponseResult<()> {
    let last_name = match &user.last_name {
        Some(l) => format!(" {}", l),
        None => String::new(),
    };
    bot.send_message(
        Recipient::Id(ChatId(*MANAGER_CHAT_ID)),
        format!(
            "{}{}:\n{}",
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
    let mut text = raw.replace("—", "--");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

async fn handle_command(text: &str) -> Result<Output, AceError> {
    let timeout = sleep(Duration::from_secs(10));

    let mut child = Command::new("bash")
        .env_remove("TELOXIDE_TOKEN")
        .env_remove("TELOXIDE_REMOVE")
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
            return Ok(output)
        }
        _ = timeout => {
            child.kill().await.expect("failed to kill, just abort");
            return Err(AceError::Timeout)
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum AceError {
    #[error("timeout: 10s elapsed")]
    Timeout,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl AceError {
    pub async fn report(
        &self,
        msg: &Message,
        bot: &AutoSend<Bot>,
    ) -> Result<(), teloxide::RequestError> {
        log::warn!("report error to chat {}: {:?}", msg.chat.id, self);
        bot.send_message(msg.chat.id, format!("{}", self))
            .reply_to_message_id(msg.id)
            .await?;
        Ok(())
    }
}
