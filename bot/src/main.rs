use std::{process::{Stdio, Output}, time::Duration};
use tokio::{process::Command, io::AsyncWriteExt, time::timeout};

use teloxide::{prelude::*, requests::ResponseResult, types::MediaKind};

#[tokio::main]
async fn main() {
    run().await;
}

async fn run() {
    teloxide::enable_logging!();
    log::info!("Starting ace-bot...");
    let bot = Bot::from_env().auto_send();
    teloxide::repl(bot, handle_update).await;
}

async fn handle_update(cx: UpdateWithCx<AutoSend<Bot>, Message>) -> ResponseResult<()> {
    match &cx.update.kind {
        teloxide::types::MessageKind::Common(message) => {
            match &message.media_kind {
                MediaKind::Text(text_media) => {
                    log::info!("handle message: {:?}", cx.update);
                    let raw_text = &text_media.text;
                    let text = preprocessing(raw_text);
                    log::info!("run command '{:?}'", text);
                    match handle_command_timeout(&text).await {
                        Err(e) => {
                            e.report(&cx).await?;
                        },
                        Ok(output) => {
                            log::info!("command '{:?}': output: {:?}", text, output);
                            let mut text_output = String::new();
                            text_output.push_str(&format!("{}", output.status));
                            if !output.stdout.is_empty() {
                                text_output.push_str(&format!("(stdout)\n{}", String::from_utf8_lossy(&output.stdout)));
                            }
                            if !output.stderr.is_empty() {
                                text_output.push_str(&format!("(stderr)\n{}", String::from_utf8_lossy(&output.stderr)));
                            }
                            if text_output.len() >= 4000 {
                                cx.reply_to("error: output message is too long").await?;
                            } else {
                                cx.reply_to(&text_output).await?;
                            }
                        }
                    };
                }
                _ => log::info!("ignored update: {:?}", cx.update)
            }
        },
        _ => log::info!("ignored update: {:?}", cx.update)
    }
    Ok(())
}

fn preprocessing(raw: &str) -> String {
    let mut text = raw.replace("—", "--");
    if !text.ends_with("\n") {
        text.push_str("\n");
    }
    text
}

async fn handle_command_timeout(text: &str) -> Result<Output, AceError> {
    timeout(Duration::from_secs(10), handle_command(text)).await?
}

async fn handle_command(text: &str) -> Result<Output, AceError> {
    let mut child = Command::new("bash")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);

    let output = child.wait_with_output().await?;
    Ok(output)
}

#[derive(thiserror::Error, Debug)]
pub enum AceError {
    #[error("timeout: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl AceError {
    pub async fn report(
        &self,
        cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    ) -> Result<(), teloxide::RequestError> {
        log::warn!("report error to chat {}: {:?}", cx.chat_id(), self);
        cx.reply_to(format!("{}", self)).await?;
        Ok(())
    }
}
