use std::ops::Deref;
use std::sync::Arc;

use log::error;
use poise::{CreateReply, ReplyHandle};
use reqwest::Client as HttpClient;
use serenity::all::{ChannelId, GuildId};
use serenity::builder::{AutocompleteChoice, CreateEmbed};
use serenity::model::Colour;
use songbird::{Call, Songbird};
use songbird::error::JoinError;
use songbird::input::YoutubeDl;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{CommandContext, Error};

const RESPONSE_COLOUR: Colour = Colour::BLURPLE;
const ERROR_COLOUR: Colour = Colour::RED;

// ======== Util functions ========

async fn get_http_client(ctx: &serenity::client::Context) -> HttpClient {
    let data = ctx.data.read().await;
    data.get::<crate::HttpKey>()
        .cloned()
        .expect("Guaranteed to exist in the typemap.")
}

fn get_author_voice_state(ctx: CommandContext<'_>) -> (GuildId, Option<ChannelId>) {
    let guild = ctx.guild().unwrap();
    let channel_id = guild
        .voice_states
        .get(&ctx.author().id)
        .and_then(|voice_state| voice_state.channel_id);

    (guild.id, channel_id)
}

fn map_err<T>(send_result: Result<T, serenity::Error>) -> Result<T, Error> {
    send_result.map_err(|e| Box::new(e) as Error)
}

async fn respond_err<'a>(
    ctx: &'a CommandContext<'a>,
    details: impl Into<String>,
) -> Result<ReplyHandle<'a>, Error> {
    let embed = CreateEmbed::new()
        .title("Fehler")
        .colour(ERROR_COLOUR)
        .field("Details", details, false);

    map_err(
        ctx.send(CreateReply::default().embed(embed).ephemeral(true))
            .await,
    )
}

// ======== Shared components ========

#[derive(Error, Debug)]
enum JoinVoiceError {
    #[error("Failed to join")]
    Join(#[from] JoinError),
    #[error("Did not join because the bot is used in another channel")]
    Occupied,
}

/// Makes the bot join a specific voice channel, if it is not already in a different one
async fn join_voice(
    songbird: impl Deref<Target = Songbird>,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<Arc<Mutex<Call>>, JoinVoiceError> {
    if let Some(call) = songbird.get(guild_id) {
        let current_channel = call.lock().await.current_channel();

        // Already in the channel
        if current_channel.is_some_and(|c| c == channel_id.into()) {
            return Ok(call);
        }

        // Used in a different channel
        if current_channel.is_some_and(|c| c != channel_id.into()) {
            return Err(JoinVoiceError::Occupied);
        }
    }

    // Bot not in a channel -> join
    Ok(songbird.join(guild_id, channel_id).await?)
}

// ======== Commands ========

/// Infos about the available commands
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    description_localized("de", "Infos zu den verfügbaren Commands")
)]
pub async fn help(ctx: CommandContext<'_>) -> Result<(), Error> {
    let listed_commands = ctx
        .framework()
        .options
        .commands
        .iter()
        .filter(|c| !c.hide_in_help);

    let embed = CreateEmbed::new()
        .title("Help")
        .colour(RESPONSE_COLOUR)
        .fields(listed_commands.map(|c| {
            (
                format!("`/{}`", c.name),
                ctx.locale()
                    .map(|l| c.description_localizations.get(l).map(|l| l.as_str()))
                    .flatten()
                    .unwrap_or(c.description.as_deref().unwrap_or_default()),
                false,
            )
        }));
    /*.field("`Weitere Infos`", "Die Warteschlange wird auch gelöscht, wenn der Bot manuell aus einem Sprachkanal entfernt wird oder den Sprachkanal wechselt\nAltersbeschränkte Videos können nicht abgespielt werden und werden automatisch übersprungen", false)
    .field("`Schnellstart`", "1. Geh in den Sprachkanal, in dem du etwas abspielen willst\n2. Spiele etwas mit /play ab", false);*/

    _ = map_err(ctx.send(CreateReply::default().embed(embed)).await)?;

    Ok(())
}

async fn autocomplete_yt_search(ctx: CommandContext<'_>, partial: &str) -> Vec<AutocompleteChoice> {
    if partial.len() < 3 || partial.starts_with("http") {
        // Discord doesn't like 0-length options
        return vec![AutocompleteChoice::new("Tippe weiter, um Suchvorschläge zu erhalten", partial)];
    }

    let http_client = get_http_client(ctx.serenity_context()).await;
    let yt_api_key = &ctx.data().yt_api_key;

    match crate::yt_search::yt_search(partial, 5, http_client, yt_api_key.as_deref()).await {
        Ok(results) => results
            .into_iter()
            .map(|video| AutocompleteChoice::new(&video.title, video.get_yt_url().as_str()))
            .collect(),
        Err(e) => {
            error!("YT search failed: {:?}", e);
            vec![AutocompleteChoice::new(partial, partial)]
        }
    }
}

//TODO: Help command text
//Spielt ein Lied im momentanen Sprachkanal ab. Als Quelle geht ein Suchbegriff für Youtube oder ein direkter Link zu allen von yt-dlp unterstützten [Platformen](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md). Standartmäßig wird das Lied hinten in die Warteschlange eingereiht. Mit skip_queue true (TAB drücken nach Commandeingabe) wird es vorne eingereiht und sofort abgespielt (überspringt das momentane Lied)

/// Plays a song in your current voice channel
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Spielt ein Lied im momentanen Sprachkanal ab")
)]
pub async fn play(
    ctx: CommandContext<'_>,
    #[description = "Youtube search or direct link to all platforms supported by yt-dlp"]
    #[description_localized(
        "de",
        "Youtube-Suche oder Direktlink zu allen von yt-dlp unterstützten Platformen"
    )]
    #[autocomplete = "autocomplete_yt_search"]
    source: String,
    #[description = "If the queue should be skipped"]
    #[description_localized("de", "Ob die Warteschlange übersprungen werden soll")]
    skip_queue: Option<bool>,
) -> Result<(), Error> {
    //TODO: Implement SkipQueue
    if skip_queue.is_some_and(|v| v) {
        _ = respond_err(&ctx, "SkipQueue ist momentan nicht verfügbar").await?;
        return Ok(());
    }

    // ======== Join the right voice channel or return ========

    // Get user's current voice channel
    let (guild_id, channel_id) = get_author_voice_state(ctx);

    // Return if user not in a voice channel
    let connect_to = match channel_id {
        Some(channel) => channel,
        None => {
            _ = respond_err(&ctx, "Du bist nicht in einem Sprachkanal in diesem Server").await?;
            return Ok(());
        }
    };

    let songbird = songbird::get(ctx.serenity_context()).await.unwrap();

    // Make sure the bot is in the right channel
    let call = match join_voice(songbird, guild_id, connect_to).await {
        Ok(c) => c,
        Err(JoinVoiceError::Join(e)) => {
            _ = respond_err(&ctx, "Der Bot konnte deinem Sprachkanal nicht beitreten").await?;
            return Err(e.into());
        }
        Err(JoinVoiceError::Occupied) => {
            _ = respond_err(
                &ctx,
                "Der Bot wird bereits in einem anderen Sprachkanal verwendet",
            )
            .await?;
            return Ok(());
        }
    };

    // ======== Play song ========

    let http_client = get_http_client(ctx.serenity_context()).await;
    let do_search = !source.starts_with("http");

    // yt-dlp lookup
    let src = if do_search {
        // This only available as a fallback for when autocomplete fails completely
        YoutubeDl::new_search(http_client, source)
    } else {
        YoutubeDl::new(http_client, source)
    };

    map_err(ctx.defer().await)?;

    // Play track
    {
        let mut call = call.lock().await;
        let _track_handle = call.enqueue_input(src.into()).await;
    }

    let embed = CreateEmbed::new()
        .title("Track Found")
        .colour(RESPONSE_COLOUR)
        .field(
            "Details",
            format!(
                "Ein Lied wird jetzt in {} abgespielt",
                ctx.guild().unwrap().channels.get(&connect_to).unwrap().name
            ),
            false,
        );
    _ = map_err(ctx.send(CreateReply::default().embed(embed)).await)?;

    Ok(())
}
