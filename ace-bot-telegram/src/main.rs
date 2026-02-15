use ace_bot::AceBot;
use ace_bot::AceError;
use ace_bot::Mode;
use ace_bot::pastebin;
use ace_bot::pastebin::curl_command;
use clap::Parser;
use futures::future::FutureExt;
use magick_rust::MagickError;
use magick_rust::MagickWand;
use magick_rust::PixelWand;
use magick_rust::magick_wand_genesis;
use magick_rust::magick_wand_terminus;
use regex::Regex;
use regex::RegexBuilder;
use std::cell::LazyCell;
use std::collections::VecDeque;
use std::fmt::Display;
use std::ops::Deref;
use std::process::Output;
use std::sync::{Arc, LazyLock};
use teloxide::RequestError;
use teloxide::types::InputFile;
use teloxide::types::InputMedia;
use teloxide::types::InputMediaAnimation;
use teloxide::types::InputMediaDocument;
use teloxide::types::InputMediaPhoto;
use teloxide::types::{ParseMode, User};
use teloxide::utils::markdown;
use teloxide::{
    prelude::*,
    requests::ResponseResult,
    types::{MediaKind, MessageKind},
    utils,
};

thread_local! {
    // cookie is not sendable across threads, so I simply use thread-local variable
    pub static COOKIE: LazyCell<magic::Cookie<magic::cookie::Load>> = LazyCell::new(|| {
        let magic_flags = magic::cookie::Flags::MIME;
        let cookie = magic::Cookie::open(magic_flags).expect("failed to open magic cookie");
        cookie.load(&magic::cookie::DatabasePaths::default()).expect("failed to load magic database")
    });
}

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
    #[arg(long, default_value_t = 600.0)]
    pub image_density: f64,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("ace error: {0}")]
    Ace(#[from] AceError),
    #[error("teloxide error: {0}")]
    Teloxide(#[from] RequestError),
    #[error("magick error: {0}")]
    Magick(#[from] MagickError),
}

static START_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^/start(@[a-zA-Z_]+)?[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static USER_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^/user(@[a-zA-Z_]+)?[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static ROOT_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^/root(@[a-zA-Z_]+)?[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static RESET_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^/reset(@[a-zA-Z_]+)?[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static NIX_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^/nix(@[a-zA-Z_]+)?[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static XELATEX_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^/xelatex(@[a-zA-Z_]+)?[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static TYPST_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^/typst(@[a-zA-Z_]+)?[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    magick_wand_genesis();

    log::info!("Starting ace-bot...");
    let options = FullOptions::parse();
    log::info!("Options = {options:#?}");
    let ctx = ArcContext(Arc::new(Context::new(options)?));
    let bot = Bot::from_env();
    let handler = Update::filter_message()
        .endpoint(handle_message)
        .branch(Update::filter_inline_query().endpoint(handle_inline_query));
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![ctx])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    magick_wand_terminus();
    Ok(())
}

async fn handle_message(ctx: ArcContext, message: Message, bot: Bot) -> Result<(), ()> {
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
                    } else if let Some(c) = XELATEX_COMMAND_PATTERN.captures(raw_text) {
                        mode = Mode::Xelatex;
                        command = c[2].to_string();
                    } else if let Some(c) = TYPST_COMMAND_PATTERN.captures(raw_text) {
                        mode = Mode::Typst;
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

async fn handle_inline_query(
    _ctx: ArcContext,
    _inline_query: InlineQuery,
    _bot: Bot,
) -> Result<(), ()> {
    // TODO working in progress
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
                    OutputMessage::format(self.clone(), &user, Some(mode), &command, output).await;
                self.handle_output(message.chat.id, bot, output_message)
                    .await
            }
        }
    }

    async fn handle_reset(self, message: Message, bot: Bot, user: User) -> ResponseResult<()> {
        match self.ace.reset().await {
            Err(e) => report_ace_error(&e, &message, &bot).await,
            Ok(output) => {
                let output_message =
                    OutputMessage::format(self.clone(), &user, None, "/reset", output).await;
                self.handle_output(message.chat.id, bot, output_message)
                    .await
            }
        }
    }

    async fn handle_output(
        self,
        chat: ChatId,
        bot: Bot,
        output: OutputMessage,
    ) -> ResponseResult<()> {
        if let Some(id) = self.options.manager_chat_id
            && ChatId(id) != chat
        {
            output.clone().send(&bot, ChatId(id)).await?;
        };
        output.send(&bot, chat).await?;
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
            photos: Default::default(),
            animations: Default::default(),
            documents: Default::default(),
        };
        help_message.send(&bot, message.chat.id).await?;
        Ok(())
    }

    fn try_magick_wand(&self) -> Result<MagickWand, Error> {
        let wand = MagickWand::new();
        let density = self.options.image_density;
        wand.set_resolution(density, density)?;
        let mut background = PixelWand::new();
        background.set_color("transparent")?; // TODO telegram does not support transparent photo
        wand.set_background_color(&background)?;
        Ok(wand)
    }

    fn magick_wand(&self) -> MagickWand {
        self.try_magick_wand()
            .unwrap_or_else(|e| panic!("failed to create MagickWand: {e}"))
    }
}

#[derive(Clone, Debug)]
pub struct OutputMessage {
    message: String,
    photos: VecDeque<InputMediaPhoto>,
    animations: VecDeque<InputMediaAnimation>,
    documents: VecDeque<InputMediaDocument>,
}

impl OutputMessage {
    async fn format(
        context: ArcContext,
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
        let mut animations = VecDeque::default();
        let mut photos = VecDeque::default();
        let mut documents = VecDeque::default();
        let client = reqwest::Client::new();

        message.push_str(&user);
        if let Some(m) = mode {
            message.push_str(&utils::markdown::escape(&format!(" ({m})")));
        } else {
            message.push_str(&utils::markdown::escape(" (meta)"));
        }
        message.push_str(":\n");
        if command.len() < PART_LIMIT {
            let language = match mode {
                None => "text",
                Some(Mode::Nix) => "nix",
                Some(Mode::Xelatex) => "tex",
                Some(Mode::Typst) => "typst",
                Some(Mode::NonRoot) | Some(Mode::Root) => "bash",
            };
            message.push_str(&utils::markdown::code_block_with_lang(
                command.trim(),
                language,
            ));
        } else {
            documents.push_back(InputMediaDocument::new(
                InputFile::memory(Vec::from(command.as_bytes())).file_name("script"),
            ));
        }
        message.push_str(&format!("{}", output.status));
        if !output.stdout.is_empty() {
            // TODO use mime to support more file formats, e.g. video, audio, etc.
            // let mime = COOKIE.with(|cookie| {
            //     cookie
            //         .buffer(&output.stdout)
            //         .unwrap_or_else(|_| "application/octet-stream".to_string())
            // });
            let wand = context.magick_wand();
            let image = if wand.read_image_blob(&output.stdout).is_ok() {
                let frame_num = wand.get_number_images();
                if frame_num > 1 {
                    wand.write_images_blob("GIF").ok().map(|data| (data, true))
                } else {
                    // static image
                    wand.write_image_blob("png").ok().map(|data| (data, false))
                }
            } else {
                None
            };
            message.push_str(&format!("\n{}", utils::markdown::escape("(stdout)")));
            if let Some((img_data, animated)) = image {
                if animated {
                    message.push_str("\nanimation attached");
                    animations.push_back(InputMediaAnimation::new(
                        InputFile::memory(img_data).file_name("stdout.gif"),
                    ));
                } else {
                    message.push_str("\nimage attached");
                    photos.push_back(InputMediaPhoto::new(
                        InputFile::memory(img_data).file_name("stdout.png"),
                    ));
                }
            } else {
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
                        documents.push_back(InputMediaDocument::new(
                            InputFile::memory(output.stdout).file_name("stdout"),
                        ));
                    } else {
                        message.push_str("\nfile size limit exceeded");
                    }
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
                    documents.push_back(InputMediaDocument::new(
                        InputFile::memory(output.stderr).file_name("stderr"),
                    ));
                } else {
                    message.push_str("\nfile size limit exceeded");
                }
            }
        }

        OutputMessage {
            message,
            animations,
            photos,
            documents,
        }
    }

    async fn send(mut self, bot: &Bot, chat_id: ChatId) -> ResponseResult<()> {
        let mut last_msg = None;

        if !self.animations.is_empty() {
            // animation can not be sent in media group
            let first = self.animations.pop_front().unwrap();
            last_msg = Some(
                bot.send_animation(chat_id, first.media)
                    .caption(self.message.clone())
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?,
            );
        }
        for a in self.animations {
            let send = bot.send_animation(chat_id, a.media);
            last_msg = Some(
                (match last_msg {
                    Some(msg) => send.reply_to_message_id(msg.id),
                    None => send
                        .caption(self.message.clone())
                        .parse_mode(ParseMode::MarkdownV2),
                })
                .await?,
            );
        }

        if !self.photos.is_empty() {
            if last_msg.is_none() {
                let first = self
                    .photos
                    .pop_front()
                    .unwrap()
                    .caption(self.message.clone())
                    .parse_mode(ParseMode::MarkdownV2);
                self.photos.push_front(first);
            }
            let media = self.photos.into_iter().map(InputMedia::Photo);
            let send = bot.send_media_group(chat_id, media);
            last_msg = Some(
                (match last_msg {
                    Some(msg) => send.reply_to_message_id(msg.id),
                    None => send,
                })
                .await?
                .into_iter()
                .next()
                .expect("empty media group response"),
            );
        }
        if !self.documents.is_empty() {
            if last_msg.is_none() {
                let first = self
                    .documents
                    .pop_front()
                    .unwrap()
                    .caption(self.message.clone())
                    .parse_mode(ParseMode::MarkdownV2);
                self.documents.push_front(first);
            }
            let media = self.documents.into_iter().map(InputMedia::Document);
            let send = bot.send_media_group(chat_id, media);
            last_msg = Some(
                (match last_msg {
                    Some(msg) => send.reply_to_message_id(msg.id),
                    None => send,
                })
                .await?
                .into_iter()
                .next()
                .expect("empty media group response"),
            );
        }
        if last_msg.is_none() {
            bot.send_message(chat_id, self.message)
                .parse_mode(ParseMode::MarkdownV2)
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
