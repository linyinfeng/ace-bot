// reference: https://github.com/matrix-org/matrix-rust-sdk/tree/main/examples/command_bot

use ace_bot::{
    AceBot, AceError, Mode,
    pastebin::{self, curl_command},
};
use clap::Parser;
use futures::future::FutureExt;
use matrix_sdk::{
    Client, ClientBuildError, Room, RoomState,
    config::SyncSettings,
    event_handler::Ctx,
    room::reply::{EnforceThread, Reply, ReplyError},
    ruma::{
        OwnedRoomId, OwnedUserId,
        events::room::{member::StrippedRoomMemberEvent, message::{
            MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
            RoomMessageEventContentWithoutRelation,
        }},
    },
};
use regex::{Regex, RegexBuilder};
use tokio::time::sleep;
use std::{
    fmt::Display,
    ops::Deref,
    process::Output,
    sync::{Arc, LazyLock}, time::Duration,
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
    options: MatrixOptions,
    client: Client,
}

impl Context {
    async fn new(options: FullOptions) -> Result<Self, Error> {
        // TODO support encryption
        let client = Client::builder()
            .homeserver_url(&options.matrix.home_server_url)
            .build()
            .await
            .map_err(Box::new)?;
        Ok(Self {
            ace: AceBot::new(options.ace)?,
            options: options.matrix,
            client,
        })
    }
}

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
struct FullOptions {
    #[command(flatten)]
    pub matrix: MatrixOptions,
    #[command(flatten)]
    pub ace: ace_bot::Options,
}

#[derive(Clone, Debug, Parser)]
#[command(author, version, about)]
struct MatrixOptions {
    #[arg(long)]
    pub home_server_url: String,
    #[arg(long)]
    pub username: String,
    #[arg(long, env = "ACE_BOT_MATRIX_PASSWORD")]
    pub password: String,
    #[arg(long)]
    pub manager_room: Option<OwnedRoomId>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("ace error: {0}")]
    Ace(#[from] AceError),
    #[error("matrix error: {0}")]
    Matrix(#[from] matrix_sdk::Error),
    #[error("reply error: {0}")]
    Reply(#[from] ReplyError),
    #[error("client build error: {0}")]
    ClientBuild(#[from] Box<ClientBuildError>),
    #[error("room not found: {0}")]
    RoomNotFound(OwnedRoomId),
}

static USER_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(!user@[a-zA-Z_]+|!user)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static ROOT_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(!root@[a-zA-Z_]+|!root)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static RESET_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(!reset@[a-zA-Z_]+|!reset)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});
static NIX_COMMAND_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new("^(!nix@[a-zA-Z_]+|!nix)[[:space:]]*(.*)$")
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    log::info!("Starting ace-bot...");
    let options = FullOptions::parse();
    log::info!("Options = {options:#?}");
    let ctx = ArcContext(Arc::new(Context::new(options).await?));
    ctx.login_and_sync().await?;
    Ok(())
}

impl ArcContext {
    async fn login_and_sync(&self) -> Result<(), Error> {
        let options = &self.options;
        let client = &self.client;
        client
            .matrix_auth()
            .login_username(&options.username, &options.password)
            .initial_device_display_name("Arbitrary Code Execution")
            .await?;
        log::info!("logged in as {}", options.username);

        let response = client.sync_once(SyncSettings::default()).await?;
        client.add_event_handler_context(self.clone());
        client.add_event_handler(Self::on_room_message);
        client.add_event_handler(Self::on_stripped_state_member);
        let settings = SyncSettings::default().token(response.next_batch);
        client.sync(settings).await?;
        Ok(())
    }

    async fn on_room_message(
        event: OriginalSyncRoomMessageEvent,
        room: Room,
        ctx: Ctx<ArcContext>,
    ) -> Result<(), Error> {
        ctx.0.handle_room_message(event, room).await
    }

    async fn on_stripped_state_member(
        room_member: StrippedRoomMemberEvent,
        client: Client,
        room: Room,
    ) {
        if room_member.state_key != client.user_id().unwrap() {
            return;
        }

        tokio::spawn(async move {
            log::info!("joining room {}", room.room_id());
            let mut delay = 2;

            while let Err(err) = room.join().await {
                // retry join due to synapse sending invites, before the
                // invited user can join for more information see
                // https://github.com/matrix-org/synapse/issues/4345
                log::warn!("failed to join room {}, retrying in {delay}s: {err}", room.room_id());

                sleep(Duration::from_secs(delay)).await;
                delay *= 2;

                if delay > 300 {
                    log::error!("failed to join room {}: {err}", room.room_id());
                    break;
                }
            }
            log::info!("joining room {}", room.room_id());
        });
    }

    async fn handle_room_message(
        self,
        event: OriginalSyncRoomMessageEvent,
        room: Room,
    ) -> Result<(), Error> {
        if room.state() != RoomState::Joined {
            return Ok(());
        }
        let MessageType::Text(text_content) = &event.content.msgtype else {
            return Ok(());
        };

        let raw_text = &text_content.body;
        let user = &event.sender;
        log::debug!("{user} raw: {raw_text}");
        if RESET_COMMAND_PATTERN.is_match(raw_text) {
            tokio::spawn(
                self.handle_reset(event.clone(), room, user.clone())
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
            command = c[2].to_string();
        } else if let Some(c) = USER_COMMAND_PATTERN.captures(raw_text) {
            mode = Mode::NonRoot;
            command = c[2].to_string();
        } else {
            log::debug!("ignored event: {event:?}");
            return Ok(());
        }
        tokio::spawn(
            self.handle_command(event.clone(), room, user.clone(), mode, command)
                .map(log_error),
        );

        Ok(())
    }

    async fn handle_command(
        self,
        event: OriginalSyncRoomMessageEvent,
        room: Room,
        user: OwnedUserId,
        mode: Mode,
        command: String,
    ) -> Result<(), Error> {
        match self.ace.run(mode, &command).await {
            Err(e) => report_ace_error(&e, &event, &room).await,
            Ok(output) => {
                let output_message =
                    OutputMessage::format(&user, Some(mode), &command, output).await;
                self.handle_output(&room, output_message).await
            }
        }
    }

    async fn handle_reset(
        self,
        event: OriginalSyncRoomMessageEvent,
        room: Room,
        user: OwnedUserId,
    ) -> Result<(), Error> {
        match self.ace.reset().await {
            Err(e) => report_ace_error(&e, &event, &room).await,
            Ok(output) => {
                let output_message = OutputMessage::format(&user, None, "/reset", output).await;
                self.handle_output(&room, output_message).await
            }
        }
    }

    async fn handle_output(self, room: &Room, output: OutputMessage) -> Result<(), Error> {
        output.send(room).await?;
        if let Some(manager_room) = self.manager_room()?
            && manager_room.room_id() != room.room_id()
        {
            output.send(&manager_room).await?;
        };
        Ok(())
    }

    fn manager_room(&self) -> Result<Option<Room>, Error> {
        let manager_room;
        if let Some(id) = &self.options.manager_room {
            match self.client.get_room(id) {
                None => return Err(Error::RoomNotFound(id.clone())),
                Some(r) => manager_room = Some(r),
            }
        } else {
            manager_room = None;
        }
        Ok(manager_room)
    }
}

#[derive(Debug)]
pub struct OutputMessage {
    message: String,
}

impl OutputMessage {
    async fn format(
        user: &OwnedUserId,
        mode: Option<Mode>,
        command: &str,
        output: Output,
    ) -> OutputMessage {
        let user = user_indicator(user);
        const PART_LIMIT: usize = 1000;
        const FILE_LIMIT: usize = 1024 * 1024; // 1 MiB

        let mut message = String::new();
        let client = reqwest::Client::new();

        message.push_str(&user);
        if let Some(m) = mode {
            message.push_str(&format!(" ({m})"));
        } else {
            message.push_str(" (meta)");
        }
        message.push_str(":\n");
        if command.len() < PART_LIMIT {
            message.push_str(command.trim());
            message.push('\n');
        } else if let Ok(cmd) =
            pastebin::curl_command(&client, "command", output.stdout.clone()).await
        {
            message.push_str(&cmd);
            message.push('\n');
        }

        message.push_str(&format!("{}", output.status));
        if !output.stdout.is_empty() {
            message.push_str(&format!("\n{}", "(stdout)"));
            let mut inlined = false;
            if let Ok(s) = String::from_utf8(output.stdout.clone())
                && s.len() < PART_LIMIT
            {
                inlined = true;
                message.push_str(&format!("\n{}", &s));
            }
            if !inlined {
                if output.stdout.len() < FILE_LIMIT {
                    if let Ok(cmd) =
                        pastebin::curl_command(&client, "stdout", output.stdout.clone()).await
                    {
                        message.push_str(&format!("\n{}", &cmd))
                    }
                } else {
                    message.push_str("\nfile size limit exceeded");
                }
            }
        }

        if !output.stderr.is_empty() {
            message.push_str(&format!("\n{}", "(stderr)"));
            let mut inlined = false;
            if let Ok(s) = String::from_utf8(output.stderr.clone())
                && s.len() < PART_LIMIT
            {
                inlined = true;
                message.push_str(&format!("\n{}", &s));
            }
            if !inlined {
                if output.stderr.len() < FILE_LIMIT {
                    if let Ok(cmd) = curl_command(&client, "stderr", output.stderr.clone()).await {
                        message.push_str(&format!("\n{}", &cmd))
                    }
                } else {
                    message.push_str("\nfile size limit exceeded");
                }
            }
        }

        OutputMessage { message }
    }

    async fn send(&self, room: &Room) -> Result<(), Error> {
        let message = RoomMessageEventContent::text_plain(&self.message);
        room.send(message).await?;
        Ok(())
    }
}

fn log_error<E: Display>(r: Result<(), E>) {
    if let Err(e) = r {
        log::warn!("error: {e}")
    }
}

fn user_indicator(user: &OwnedUserId) -> String {
    format!("{user}")
}

pub async fn report_ace_error(
    err: &AceError,
    event: &OriginalSyncRoomMessageEvent,
    room: &Room,
) -> Result<(), Error> {
    log::warn!("report error to room {}: {:?}", room.room_id(), err);
    reply(event, room, &format!("{err}")).await
}

pub async fn reply(
    event: &OriginalSyncRoomMessageEvent,
    room: &Room,
    text: &str,
) -> Result<(), Error> {
    let message = RoomMessageEventContentWithoutRelation::text_plain(text);
    let reply = Reply {
        event_id: event.event_id.clone(),
        enforce_thread: EnforceThread::MaybeThreaded,
    };
    let reply_event = room.make_reply_event(message, reply).await?;
    room.send(reply_event).await?;
    Ok(())
}
