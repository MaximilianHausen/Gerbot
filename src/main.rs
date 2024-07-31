use std::collections::HashMap;
use std::env;

use log::{error, info, LevelFilter};
use poise::FrameworkContext;
use reqwest::Client as HttpClient;
use serenity::all::GuildId;
use serenity::Client;
use serenity::client::FullEvent;
use serenity::prelude::*;
use songbird::SerenityInit;

mod music_commands;
pub mod yt_search;

// Types used by all command functions
type Error = Box<dyn std::error::Error + Send + Sync>;
type CommandContext<'a> = poise::Context<'a, Data, Error>;

struct HttpKey;

impl TypeMapKey for HttpKey {
    type Value = HttpClient;
}

// Custom user data passed to all command functions
pub struct Data {
    yt_api_key: Option<String>,
    per_guild_data: Mutex<HashMap<GuildId, ()>>,
}

async fn on_poise_error(error: poise::FrameworkError<'_, Data, Error>) {
    if let Err(e) = poise::builtins::on_error(error).await {
        error!("Error while handling error: {}", e);
    }
}

async fn on_api_event(
    ctx: &Context,
    event: &FullEvent,
    framework: FrameworkContext<'_, Data, Error>,
    _data: &Data,
) -> Result<(), Error> {
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
            let songbird = songbird::get(ctx).await.unwrap();

            let guild_id = match new.guild_id {
                Some(guild_id) => guild_id,
                None => return Ok(()),
            };

            let call_lock = match songbird.get(guild_id) {
                Some(call_clock) => call_clock,
                None => return Ok(()),
            };
            let mut call = call_lock.lock().await;

            // Check if the bot is the only one left in its channel
            let should_leave = call.current_channel().is_some_and(|channel_id| {
                let guild = ctx.cache.guild(guild_id).unwrap();

                !guild.voice_states.iter().any(|(_, state)| {
                    state.channel_id == Some(channel_id.0.into())
                        && state.user_id != framework.bot_id
                })
            });

            if should_leave {
                call.queue().stop();
                call.stop();
                call.leave()
                    .await
                    .expect("Failed to leave channel after last user");
            }
        }
        _ => {}
    };
    Ok(())
}

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
        commands: vec![music_commands::help(), music_commands::play()],
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
                Ok(Data {
                    yt_api_key: std::env::var("YOUTUBE_API_KEY").ok(),
                    per_guild_data: Mutex::new(HashMap::new()),
                })
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
        .await
        .expect("Err creating client");

    // Start client
    client.start().await.unwrap();
}
