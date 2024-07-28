mod music_commands;

use log::{error, info};
use reqwest::Client as HttpClient;
use serenity::all::GuildId;
use serenity::prelude::*;
use serenity::Client;
use songbird::tracks::TrackHandle;
use songbird::SerenityInit;
use std::collections::HashMap;
use std::env;

// Types used by all command functions
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

struct HttpKey;

impl TypeMapKey for HttpKey {
    type Value = HttpClient;
}

pub struct GuildData {
    current_track: TrackHandle,
}

// Custom user data passed to all command functions
pub struct Data {
    per_guild_data: Mutex<HashMap<GuildId, GuildData>>,
}

async fn on_poise_error(error: poise::FrameworkError<'_, Data, Error>) {
    if let Err(e) = poise::builtins::on_error(error).await {
        error!("Error while handling error: {}", e);
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let token = env::var("DISCORD_TOKEN").expect("Missing `DISCORD_TOKEN` env var");

    // FrameworkOptions contains all of poise's configuration option in one struct
    // Every option can be omitted to use its default value
    let options = poise::FrameworkOptions {
        commands: vec![music_commands::help(), music_commands::play()],
        // The global error handler for all error cases that may occur
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
        ..Default::default()
    };

    let framework = poise::Framework::builder()
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                println!("Logged in as {}", _ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    per_guild_data: Mutex::new(HashMap::new()),
                })
            })
        })
        .options(options)
        .build();

    // Create a new instance of the Client, logging in as a bot.
    let mut client = Client::builder(&token, GatewayIntents::empty())
        .intents(GatewayIntents::non_privileged())
        .framework(framework)
        .register_songbird()
        .type_map_insert::<HttpKey>(HttpClient::new())
        .await
        .expect("Err creating client");

    // Start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("Client error: {why:?}");
    }
}
