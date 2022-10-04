use once_cell::sync::Lazy;
use std::{
    process::{Output, Stdio},
    time::Duration,
};
use teloxide::{prelude::*, requests::ResponseResult, types::MediaKind};
use teloxide::types::User;
use tokio::{io::AsyncWriteExt, process::Command};
use tokio::select;
use tokio::time::sleep;

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
    teloxide::enable_logging!();
    log::info!("Starting ace-bot...");
    let bot = Bot::from_env().auto_send();
    teloxide::repl(bot, handle_update).await;
}

async fn handle_update(cx: UpdateWithCx<AutoSend<Bot>, Message>) -> ResponseResult<()> {
    match &cx.update.kind {
        teloxide::types::MessageKind::Common(message) => match &message.media_kind {
            MediaKind::Text(text_media) => match &message.from {
                Some(user) => {
                    log::info!("handle message: {:?}", cx.update);
                    let raw_text = &text_media.text;
                    let text = preprocessing(raw_text);
                    log::info!("{:?}: {}", user, text);
                    log_for_manager(&cx.requester, &user, &text).await?;
                    match handle_command(&text).await {
                        Err(e) => {
                            e.report(&cx).await?;
                        }
                        Ok(output) => {
                            log::info!("command '{:?}': output: {:?}", text, output);
                            let mut text_output = String::new();
                            text_output.push_str(&format!("{}", output.status));
                            if !output.stdout.is_empty() {
                                text_output.push_str(&format!(
                                    "\n(stdout)\n{}",
                                    String::from_utf8_lossy(&output.stdout)
                                ));
                            }
                            if !output.stderr.is_empty() {
                                text_output.push_str(&format!(
                                    "\n(stderr)\n{}",
                                    String::from_utf8_lossy(&output.stderr)
                                ));
                            }
                            if text_output.len() >= 4000 {
                                cx.reply_to("error: output message is too long").await?;
                            } else {
                                cx.reply_to(&text_output).await?;
                            }
                        }
                    }
                },
                _ => log::info!("ignored update: {:?}", cx.update),
            }
            _ => log::info!("ignored update: {:?}", cx.update),
        },
        _ => log::info!("ignored update: {:?}", cx.update),
    }
    Ok(())
}

async fn log_for_manager(requester: &AutoSend<Bot>, user: &User, text: &str) -> ResponseResult<()> {
    let last_name = match &user.last_name {
        Some(l) => format!(" {}", l),
        None => String::new(),
    };
    requester.send_message(*MANAGER_CHAT_ID, format!("{}{}:\n{}", user.first_name, last_name, text)).await?;
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
        .env_clear() // clear TELOXIDE_*
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
        cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    ) -> Result<(), teloxide::RequestError> {
        log::warn!("report error to chat {}: {:?}", cx.chat_id(), self);
        cx.reply_to(format!("{}", self)).await?;
        Ok(())
    }
}
