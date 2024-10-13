use crate::music_commands::{GetCallError, JoinVoiceError};
use crate::youtube::YoutubeClient;
use log::{error, info, warn, LevelFilter};
use poise::{CreateReply, FrameworkContext, FrameworkError};
use reqwest::Client as HttpClient;
use serenity::all::{Colour, CreateEmbed};
use serenity::client::FullEvent;
use serenity::prelude::*;
use serenity::Client;
use songbird::SerenityInit;
use std::env;
use thiserror::Error;

mod metadata;
mod music_commands;
mod serde;
mod youtube;

const SUCCESS_COLOUR: Colour = Colour::BLURPLE;
const ERROR_COLOUR: Colour = Colour::RED;

// Types used by all command functions
type CommandContext<'a> = poise::Context<'a, GlobalData, CommandError>;

#[derive(Error, Debug)]
enum CommandError {
    #[error("Serenity error")]
    Serenity(#[from] SerenityError),
    #[error("Failed to join a voice channel")]
    JoinVoice(#[from] JoinVoiceError),
    #[error("Failed to leave a voice channel")]
    LeaveVoice,
    #[error("Guild-only command executed from DMs. This should have been caught by poise")]
    NotInGuild,
    #[error("Songbird instance could not be retrieved from the command context")]
    SongbirdNotFound,
    #[error("The author is not in a voice channel")]
    UserNotInVoice,
    #[error("The author is not in a voice channel with the bot")]
    NotInCall,
    #[error("No track is currently playing")]
    QueueEmpty,
}

impl From<GetCallError> for CommandError {
    fn from(value: GetCallError) -> Self {
        match value {
            GetCallError::NotInGuild => CommandError::NotInGuild,
            GetCallError::SongbirdNotFound => CommandError::SongbirdNotFound,
            GetCallError::NotInCall => CommandError::NotInCall,
        }
    }
}

struct HttpKey;

impl TypeMapKey for HttpKey {
    type Value = HttpClient;
}

struct YoutubeKey;

impl TypeMapKey for YoutubeKey {
    type Value = YoutubeClient;
}

// Custom user data passed to all command functions
pub struct GlobalData {}

#[tokio::main]
async fn main() {
    if env::var("RUST_LOG").is_ok() {
        env_logger::init();
    } else {
        env_logger::Builder::new()
            .filter(Some("gerbot"), LevelFilter::Info)
            .init();
    }

    let token = env::var("DISCORD_TOKEN").expect("Missing `DISCORD_TOKEN` env var");

    // Create framework configuration
    let options = poise::FrameworkOptions {
        commands: vec![
            music_commands::help(),
            music_commands::play(),
            music_commands::now_playing(),
            music_commands::queue(),
            music_commands::loop_command(),
            music_commands::skip(),
            music_commands::stop(),
            music_commands::leave(),
        ],
        on_error: |error| Box::pin(on_poise_error(error)),
        // This code is run before every command
        pre_command: |ctx| {
            Box::pin(async move {
                info!(
                    "Executing command {} for user {}<{}>...",
                    ctx.command().qualified_name,
                    ctx.author().name,
                    ctx.author().id
                );
            })
        },
        event_handler: |ctx, event, framework, data| {
            Box::pin(on_api_event(ctx, event, framework, data))
        },
        ..Default::default()
    };

    // Build framework
    let framework = poise::Framework::builder()
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(GlobalData {})
            })
        })
        .options(options)
        .build();

    // Create client config
    let mut client = Client::builder(&token, GatewayIntents::empty())
        .intents(GatewayIntents::non_privileged())
        .framework(framework)
        .register_songbird()
        .type_map_insert::<HttpKey>(HttpClient::new())
        .type_map_insert::<YoutubeKey>(YoutubeClient::new(
            HttpClient::new(),
            std::env::var("YOUTUBE_API_KEY").ok(),
        ))
        .await
        .expect("Error creating client");

    // Start client
    client.start().await.unwrap();
}

async fn on_api_event(
    ctx: &Context,
    event: &FullEvent,
    framework: FrameworkContext<'_, GlobalData, CommandError>,
    _data: &GlobalData,
) -> Result<(), CommandError> {
    match event {
        FullEvent::CacheReady { guilds } => {
            // Print startup info
            info!("Logged in as {}", ctx.cache.current_user().name);
            if guilds.len() < 10 {
                info!(
                    "Active on these guilds: {}",
                    guilds
                        .iter()
                        .map(|g| format!("{}<{}>", ctx.cache.guild(g).unwrap().name, g))
                        .collect::<Vec<String>>()
                        .join(", ")
                );
            } else {
                info!("Active on {} guilds", guilds.len())
            }
        }
        // Leave empty voice channels automatically
        FullEvent::VoiceStateUpdate { new, .. } => {
            let songbird = songbird::get(ctx)
                .await
                .ok_or(CommandError::SongbirdNotFound)?;

            let Some(guild_id) = new.guild_id else {
                error!("The bot is in a private call (How???)");
                return Ok(());
            };

            let Some(call_lock) = songbird.get(guild_id) else {
                // Not in a call
                return Ok(());
            };
            let mut call = call_lock.lock().await;

            // Clear queue when forcefully disconnected by a moderator
            if call.current_channel().is_none() && !call.queue().is_empty() {
                info!(
                    "Bot got disconnected from a voice channel in guild {}",
                    guild_id
                );
                call.queue().stop();
                call.stop();
            }

            // Check if the bot is the only one left in its channel
            let should_leave = call.current_channel().is_some_and(|channel_id| {
                let guild = guild_id.to_guild_cached(ctx).expect("Guild not in cache");

                !guild.voice_states.iter().any(|(_, state)| {
                    state.channel_id == Some(channel_id.0.into())
                        && state.user_id != framework.bot_id
                })
            });

            if should_leave {
                call.queue().stop();
                call.stop();
                call.leave().await.map_err(|_| CommandError::LeaveVoice)?;
            }
        }
        _ => {}
    };

    Ok(())
}

// ======== Error handling ========

async fn respond_err(ctx: &CommandContext<'_>, details: impl Into<String>) {
    let embed = CreateEmbed::new()
        .title("Fehler")
        .colour(ERROR_COLOUR)
        .field("Details", details, false);

    if let Err(e) = ctx
        .send(CreateReply::default().embed(embed).ephemeral(true))
        .await
    {
        error!("Error while sending error response: {}", e);
    }
}

async fn handle_command_error(ctx: &CommandContext<'_>, error: CommandError) {
    match error {
        CommandError::Serenity(inner) => {
            error!("Serenity error: {}", inner);
            respond_err(ctx, "Ein unerwarteter Fehler ist aufgetreten").await;
        }
        CommandError::JoinVoice(inner) => match inner {
            JoinVoiceError::Join(inner) => {
                error!("Failed to join voice channel: {}", inner);
                respond_err(ctx, "Der Bot konnte deinem Sprachkanal nicht beitreten").await;
            }
            JoinVoiceError::Occupied => {
                respond_err(
                    ctx,
                    "Der Bot wird bereits in einem anderen Sprachkanal verwendet",
                )
                .await;
            }
        },
        CommandError::LeaveVoice => {
            error!("Failed to leave voice channel: {}", error);
            respond_err(ctx, "Ein unerwarteter Fehler ist aufgetreten").await;
        }
        CommandError::NotInGuild => {
            // This should never happen as it is caught by the poise attribute
        }
        CommandError::SongbirdNotFound => {
            error!("Songbird instance could not be retrieved from the typemap");
            respond_err(ctx, "Ein unerwarteter Fehler ist aufgetreten").await;
        }
        CommandError::UserNotInVoice => {
            respond_err(ctx, "Du bist nicht in einem Sprachkanal in diesem Server").await;
        }
        CommandError::NotInCall => {
            respond_err(ctx, "Du bist nicht in einem Sprachkanal mit dem Bot").await;
        }
        CommandError::QueueEmpty => respond_err(ctx, "Momentan wird nichts abgespielt").await,
    }
}

async fn on_poise_error(error: poise::FrameworkError<'_, GlobalData, CommandError>) {
    match error {
        FrameworkError::Setup { error, .. } => error!("Error in data setup: {}", error),
        FrameworkError::EventHandler { error, event, .. } => {
            error!(
                "Error in {} event handler: {}",
                event.snake_case_name(),
                error
            )
        }
        FrameworkError::Command { ctx, error, .. } => {
            handle_command_error(&ctx, error).await;
        }
        FrameworkError::CommandPanic { ctx, payload, .. } => {
            match payload {
                Some(payload) => error!("Command panicked. Details:\n{}", payload),
                None => error!("Command panicked"),
            }
            respond_err(&ctx, "Ein unerwarteter Fehler ist aufgetreten").await;
        }
        FrameworkError::ArgumentParse {
            ctx, input, error, ..
        } => {
            let msg = match input {
                Some(arg) => {
                    error!("Error while parsing command argument {arg}: {error}");
                    format!("Fehler beim Lesen des Command-Arguments {arg}")
                }
                None => {
                    error!("Error while parsing command arguments: {error}");
                    "Fehler beim Lesen eines Command-Arguments".to_owned()
                }
            };

            respond_err(&ctx, msg).await;
        }
        FrameworkError::CommandStructureMismatch {
            ctx, description, ..
        } => {
            error!(
                "Failed to deserialize interaction for /{} (Maybe the command hasn't fully updated?): {}",
                ctx.command.name, description,
            );
            let msg = "Ein unerwarteter Fehler ist aufgetreten. Du kannst versuchen, Discord neu zu starten oder ein paar Minuten zu warten.";
            respond_err(&CommandContext::Application(ctx), msg).await;
        }
        FrameworkError::CooldownHit {
            ctx,
            remaining_cooldown,
            ..
        } => {
            let msg = format!(
                "Nicht so schnell. Bitte warte {} Sekunden vor dem nächsten Versuch",
                remaining_cooldown.as_secs()
            );
            respond_err(&ctx, msg).await;
        }
        FrameworkError::MissingBotPermissions {
            ctx,
            missing_permissions,
            ..
        } => {
            let msg = format!(
                "Der Command konnte nicht ausgeführt werden, weil dem Bot folgende Berechtigungen fehlen: {}",
                missing_permissions,
            );
            respond_err(&ctx, msg).await;
        }
        FrameworkError::NotAnOwner { ctx, .. } => {
            let msg = "Dieser Command kann nur von den Besitzern des Bots verwendet werden";
            respond_err(&ctx, msg).await;
        }
        FrameworkError::GuildOnly { ctx, .. } => {
            let msg = "Dieser Command kann nur in einem Server verwendet werden";
            respond_err(&ctx, msg).await;
        }
        FrameworkError::DmOnly { ctx, .. } => {
            let msg = "Dieser Command kann nur in DMs verwendet werden";
            respond_err(&ctx, msg).await;
        }
        FrameworkError::NsfwOnly { ctx, .. } => {
            let msg = "Dieser Command kann nur in NSFW Kanälen verwendet werden";
            respond_err(&ctx, msg).await;
        }
        FrameworkError::CommandCheckFailed { ctx, error, .. } => match error {
            Some(e) => {
                handle_command_error(&ctx, e).await;
            }
            None => {
                respond_err(&ctx, "Der Command wurde abgebrochen").await;
            }
        },
        FrameworkError::UnknownInteraction { interaction, .. } => {
            warn!("Unknown interaction received: {:?}", interaction);
        }
        _ => {
            // Anything else is only relevant for prefix commands
        }
    }
}
