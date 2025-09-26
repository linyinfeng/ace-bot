use ace_bot::AceBot;
use ace_bot::AceError;
use ace_bot::Mode;
use ace_bot::pastebin;
use ace_bot::pastebin::curl_command;
use clap::Parser;
use futures::future::FutureExt;
use regex::Regex;
use regex::RegexBuilder;
use std::fmt::Display;
use std::ops::Deref;
use std::process::Output;
use std::sync::{Arc, LazyLock};
use teloxide::RequestError;
use teloxide::types::InputFile;
use teloxide::types::InputMedia;
use teloxide::types::InputMediaDocument;
use teloxide::types::{ParseMode, User};
use teloxide::utils::markdown;
use teloxide::{
    prelude::*,
    requests::ResponseResult,
    types::{MediaKind, MessageKind},
    utils,
};

#[derive(Debug, Clone)]
struct ArcContext(Arc<Context>);
impl Deref for ArcContext {
    type Target = Context;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
struct Context {
    ace: AceBot,
    options: TgOptions,
}

impl Context {
    fn new(options: FullOptions) -> Result<Self, Error> {
        Ok(Self {
            ace: AceBot::new(options.ace)?,
            options: options.tg,
        })
    }
}

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
struct FullOptions {
    #[command(flatten)]
    pub tg: TgOptions,
    #[command(flatten)]
    pub ace: ace_bot::Options,
}

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
struct TgOptions {
    #[arg(long)]
    pub manager_chat_id: Option<i64>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("ace error: {0}")]
    Ace(#[from] AceError),
    #[error("teloxide error: {0}")]
    Teloxide(#[from] RequestError),
}

static START_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(/start@[a-zA-Z_]+|/start)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static USER_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(/user@[a-zA-Z_]+|/user)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static ROOT_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(/root@[a-zA-Z_]+|/root)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static RESET_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(/reset@[a-zA-Z_]+|/reset)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static NIX_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(/nix@[a-zA-Z_]+|/nix)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run().await
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    log::info!("Starting ace-bot...");
    let options = FullOptions::parse();
    log::info!("Options = {options:#?}");
    let ctx = ArcContext(Arc::new(Context::new(options)?));
    let bot = Bot::from_env();
    Dispatcher::builder(bot, Update::filter_message().endpoint(handle_update))
        .dependencies(dptree::deps![ctx])
        .build()
        .dispatch()
        .await;
    Ok(())
}

async fn handle_update(ctx: ArcContext, message: Message, bot: Bot) -> Result<(), ()> {
    match &message.kind {
        MessageKind::Common(common_msg) => match &common_msg.media_kind {
            MediaKind::Text(text_media) => match &common_msg.from {
                Some(user) => {
                    let raw_text = &text_media.text;
                    log::debug!("{user:?} raw: {raw_text}");
                    if START_COMMAND_PATTERN.is_match(raw_text) {
                        tokio::spawn(
                            ctx.handle_start(message.clone(), bot.clone())
                                .map(log_error),
                        );
                        return Ok(());
                    }
                    if RESET_COMMAND_PATTERN.is_match(raw_text) {
                        tokio::spawn(
                            ctx.handle_reset(message.clone(), bot.clone(), user.clone())
                                .map(log_error),
                        );
                        return Ok(());
                    }
                    let mode;
                    let command;
                    if let Some(c) = NIX_COMMAND_PATTERN.captures(raw_text) {
                        mode = Mode::Nix;
                        command = c[2].to_string();
                    } else if let Some(c) = ROOT_COMMAND_PATTERN.captures(raw_text) {
                        mode = Mode::Root;
                        command = preprocessing(&c[2]);
                    } else if let Some(c) = USER_COMMAND_PATTERN.captures(raw_text) {
                        mode = Mode::NonRoot;
                        command = preprocessing(&c[2]);
                    } else if message.chat.id.is_user() {
                        mode = Mode::NonRoot;
                        command = preprocessing(raw_text);
                    } else {
                        log::debug!("ignored update: {message:?}");
                        return Ok(());
                    }
                    tokio::spawn(
                        ctx.handle_command(message.clone(), bot, user.clone(), mode, command)
                            .map(log_error),
                    );
                }
                _ => log::debug!("ignored update: {message:?}"),
            },
            _ => log::debug!("ignored update: {message:?}"),
        },
        _ => log::debug!("ignored update: {message:?}"),
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

fn log_error<E: Display>(r: Result<(), E>) {
    if let Err(e) = r {
        log::warn!("error: {e}")
    }
}

impl ArcContext {
    async fn handle_command(
        self,
        message: Message,
        bot: Bot,
        user: User,
        mode: Mode,
        command: String,
    ) -> ResponseResult<()> {
        match self.ace.run(mode, &command).await {
            Err(e) => report_ace_error(&e, &message, &bot).await,
            Ok(output) => {
                let output_message =
                    OutputMessage::format(&user, Some(mode), &command, output).await;
                self.handle_output(message, bot, output_message).await
            }
        }
    }

    async fn handle_reset(self, message: Message, bot: Bot, user: User) -> ResponseResult<()> {
        match self.ace.reset().await {
            Err(e) => report_ace_error(&e, &message, &bot).await,
            Ok(output) => {
                let output_message = OutputMessage::format(&user, None, "/reset", output).await;
                self.handle_output(message, bot, output_message).await
            }
        }
    }

    async fn handle_output(
        self,
        message: Message,
        bot: Bot,
        output: OutputMessage,
    ) -> ResponseResult<()> {
        output.send(&bot, message.chat.id).await?;
        if let Some(id) = self.options.manager_chat_id
            && ChatId(id) != message.chat.id
        {
            output.send(&bot, ChatId(id)).await?;
        };
        Ok(())
    }

    async fn handle_start(self, message: Message, bot: Bot) -> ResponseResult<()> {
        let help_message = OutputMessage {
            message: "hello, world
    ```
    /user  - run bash commands as a normal user
    /root  - run bash commands as a root user
    /reset - reset the whole environment
    ```"
            .to_string(),
            documents: vec![],
        };
        help_message.send(&bot, message.chat.id).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct OutputMessage {
    message: String,
    documents: Vec<InputMedia>,
}

impl OutputMessage {
    async fn format(
        user: &User,
        mode: Option<Mode>,
        command: &str,
        output: Output,
    ) -> OutputMessage {
        // TODO wait for https://github.com/teloxide/teloxide/pull/1411
        let user = match user.mention() {
            Some(mention) => markdown::escape(&mention),
            None => markdown::link(user.url().as_str(), &markdown::escape(&user.full_name())),
        };
        const PART_LIMIT: usize = 1000;
        const FILE_LIMIT: usize = 1024 * 1024; // 1 MiB

        let mut message = String::new();
        let mut documents = Vec::new();
        let client = reqwest::Client::new();

        message.push_str(&user);
        if let Some(m) = mode {
            message.push_str(&utils::markdown::escape(&format!(" ({m})")));
        } else {
            message.push_str(&utils::markdown::escape(" (meta)"));
        }
        message.push_str(":\n");
        if command.len() < PART_LIMIT {
            message.push_str(&utils::markdown::code_block(command.trim()));
        } else {
            documents.push(InputMedia::Document(InputMediaDocument::new(
                InputFile::memory(Vec::from(command.as_bytes())).file_name("script"),
            )));
        }
        message.push_str(&format!("{}", output.status));
        if !output.stdout.is_empty() {
            message.push_str(&format!("\n{}", utils::markdown::escape("(stdout)")));
            let mut inlined = false;
            if let Ok(s) = String::from_utf8(output.stdout.clone())
                && s.len() < PART_LIMIT
            {
                inlined = true;
                message.push_str(&format!("\n{}", utils::markdown::code_block(&s)));
            }
            if !inlined {
                if output.stdout.len() < FILE_LIMIT {
                    message.push_str("\nattached");
                    if let Ok(cmd) =
                        pastebin::curl_command(&client, "stdout", output.stdout.clone()).await
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
            if let Ok(s) = String::from_utf8(output.stderr.clone())
                && s.len() < PART_LIMIT
            {
                inlined = true;
                message.push_str(&format!("\n{}", utils::markdown::code_block(&s)));
            }
            if !inlined {
                if output.stderr.len() < FILE_LIMIT {
                    message.push_str("\nattached");
                    if let Ok(cmd) = curl_command(&client, "stderr", output.stderr.clone()).await {
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

pub async fn report_ace_error(
    err: &AceError,
    msg: &Message,
    bot: &Bot,
) -> Result<(), teloxide::RequestError> {
    log::warn!("report error to chat {}: {:?}", msg.chat.id, err);
    bot.send_message(msg.chat.id, format!("{err}"))
        .reply_to_message_id(msg.id)
        .await?;
    Ok(())
}
