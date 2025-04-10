use log::error;
use poise::{CreateReply, ReplyHandle};
use rand::prelude::SliceRandom;
use reqwest::{Client as HttpClient, Url};
use serenity::all::{ChannelId, GuildId};
use serenity::builder::{AutocompleteChoice, CreateAllowedMentions, CreateEmbed};
use serenity::futures::future::join_all;
use serenity::prelude::Mentionable;
use songbird::error::JoinError;
use songbird::input::{Compose, YoutubeDl};
use songbird::tracks::{LoopState, Track};
use songbird::{Call, Songbird};
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::metadata::TrackMetadata;
use crate::music_commands::GetCallError::{NotInCall, NotInGuild, SongbirdNotFound};
use crate::youtube::{YoutubeClient, YtResourceId, YtSearchFilter};
use crate::CommandError::{LeaveVoice, QueueEmpty, UserNotInVoice};
use crate::{CommandContext, CommandError, SUCCESS_COLOUR};

// ======== Util functions ========

async fn get_http_client(ctx: &serenity::client::Context) -> HttpClient {
    let data = ctx.data.read().await;
    data.get::<crate::HttpKey>()
        .cloned()
        .expect("Guaranteed to exist in the typemap")
}

async fn get_youtube_client(ctx: &serenity::client::Context) -> YoutubeClient {
    let data = ctx.data.read().await;
    data.get::<crate::YoutubeKey>()
        .cloned()
        .expect("Guaranteed to exist in the typemap")
}

fn get_author_voice_state(ctx: CommandContext<'_>) -> (GuildId, Option<ChannelId>) {
    let guild = ctx.guild().expect("Guild not in cache");
    let channel_id = guild
        .voice_states
        .get(&ctx.author().id)
        .and_then(|voice_state| voice_state.channel_id);

    (guild.id, channel_id)
}

struct YtUrlIds {
    video_id: Option<String>,
    playlist_id: Option<String>,
}

fn get_yt_id_from_url(url: &str) -> YtUrlIds {
    //TODO: Sanitize parsed yt ids
    match Url::parse(url).ok() {
        Some(url) if url.domain().is_some_and(|d| d == "youtu.be") => YtUrlIds {
            video_id: Some(url.path()[1..].to_owned()),
            playlist_id: None,
        },
        Some(url) if url.domain().is_some_and(|d| d.ends_with("youtube.com")) => YtUrlIds {
            video_id: url
                .query_pairs()
                .filter_map(|(k, v)| (k == "v").then_some((*v).to_owned()))
                .next(),
            playlist_id: url
                .query_pairs()
                .filter_map(|(k, v)| (k == "list").then_some((*v).to_owned()))
                .next(),
        },
        _ => YtUrlIds {
            video_id: None,
            playlist_id: None,
        },
    }
}

// ======== Shared components ========

async fn respond_success<'a>(
    ctx: &'a CommandContext<'a>,
    title: impl Into<String>,
    details: impl Into<String>,
    ephemeral: bool,
) -> Result<ReplyHandle<'a>, serenity::Error> {
    let embed = CreateEmbed::new()
        .title(title)
        .colour(SUCCESS_COLOUR)
        .description(details);

    ctx.send(
        CreateReply::default()
            .embed(embed)
            .ephemeral(ephemeral)
            .allowed_mentions(CreateAllowedMentions::new().empty_users()),
    )
    .await
}

#[derive(Error, Debug)]
pub enum JoinVoiceError {
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

#[derive(Error, Debug)]
pub enum GetCallError {
    #[error("The command was not called from a guild (this should never happen)")]
    NotInGuild,
    #[error("Songbird instance could not be retrieved from the command context")]
    SongbirdNotFound,
    #[error("The author is not in a voice channel with the bot")]
    NotInCall,
}

/// Shared boilerplate for getting the active call for a command and correctly mapping all the error cases
async fn get_call(ctx: CommandContext<'_>) -> Result<(ChannelId, Arc<Mutex<Call>>), GetCallError> {
    let guild_id = ctx.guild_id().ok_or(NotInGuild)?;
    let songbird = songbird::get(ctx.serenity_context())
        .await
        .ok_or(SongbirdNotFound)?;
    let call = songbird.get(guild_id).ok_or(NotInCall)?;
    let bot_channel = call.lock().await.current_channel().ok_or(NotInCall)?;
    let user_channel = get_author_voice_state(ctx).1.ok_or(NotInCall)?;

    if bot_channel != user_channel.into() {
        return Err(NotInCall);
    }

    Ok((user_channel, call))
}

async fn enqueue_track(
    ctx: CommandContext<'_>,
    call: Arc<Mutex<Call>>,
    source: &str,
) -> Result<Arc<TrackMetadata>, CommandError> {
    let http_client = get_http_client(ctx.serenity_context()).await;
    let youtube_client = get_youtube_client(ctx.serenity_context()).await;

    let url = Url::parse(source).ok();
    // Extract youtube video id from url
    let youtube_id = url
        .as_ref()
        .and_then(|url| get_yt_id_from_url(url.as_ref()).video_id);

    let mut track = if let Some(url) = url {
        YoutubeDl::new(http_client.clone(), url.to_string())
    } else {
        // This only available as a fallback for when autocomplete fails completely
        YoutubeDl::new_search(http_client.clone(), source.to_owned())
    };

    let metadata = match youtube_id {
        Some(video_id) => Arc::new(TrackMetadata::from_with_request(
            youtube_client
                .get_video(&video_id)
                .await
                .map(TrackMetadata::from)
                .unwrap_or_default(),
            ctx.author().id,
        )),
        None => Arc::new(TrackMetadata::from_with_request(
            track
                .aux_metadata()
                .await
                .map(TrackMetadata::from)
                .unwrap_or_default(),
            ctx.author().id,
        )),
    };

    let mut call = call.lock().await;
    call.enqueue_with_preload(
        Track::new_with_data(track.into(), metadata.clone()),
        Some(metadata.duration.saturating_sub(Duration::from_secs(5))),
    );

    Ok(metadata)
}

// ======== Commands ========

/// Infos about the available commands
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Infos zu den verfügbaren Commands")
)]
pub async fn help(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let listed_commands = ctx
        .framework()
        .options
        .commands
        .iter()
        .filter(|c| !c.hide_in_help);

    let embed = CreateEmbed::new()
        .title("Help")
        .colour(SUCCESS_COLOUR)
        .fields(listed_commands.map(|c| {
            (
                format!("`/{}`", c.name),
                ctx.locale()
                    .and_then(|l| c.description_localizations.get(l).map(|l| l.as_str()))
                    .unwrap_or(c.description.as_deref().unwrap_or_default()),
                false,
            )
        }));
    //.field("`Weitere Infos`", "Die Warteschlange wird auch gelöscht, wenn der Bot manuell aus einem Sprachkanal entfernt wird oder den Sprachkanal wechselt", false)

    _ = ctx
        .send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;

    Ok(())
}

async fn autocomplete_yt_video_search(
    ctx: CommandContext<'_>,
    partial: &str,
) -> Vec<AutocompleteChoice> {
    if partial.len() < 3 {
        // Discord doesn't like 0-length options
        return vec![AutocompleteChoice::new(
            "Tippe weiter, um Suchvorschläge zu erhalten",
            partial,
        )];
    }

    let youtube_client = get_youtube_client(ctx.serenity_context()).await;

    // YouTube URL
    if let Some(id) = get_yt_id_from_url(partial).video_id {
        return match youtube_client.get_video(&id).await {
            Ok(video) => vec![AutocompleteChoice::new(video.title, partial)],
            Err(e) => {
                error!("YT video lookup for id {} failed: {:?}", id, e);
                vec![AutocompleteChoice::new(partial, partial)]
            }
        };
    }

    // Other URL (include ':' to allow searches that start with "http")
    if partial.starts_with("https:") || partial.starts_with("http:") {
        return vec![AutocompleteChoice::new(partial, partial)];
    }

    // Random text -> search
    match youtube_client
        .search(partial, YtSearchFilter::Videos, 5)
        .await
    {
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
//Spielt ein Lied im momentanen Sprachkanal ab. Als Quelle geht ein Suchbegriff für Youtube oder ein direkter Link zu allen von yt-dlp unterstützten [Platformen](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md). Standardmäßig wird das Lied hinten in die Warteschlange eingereiht. Mit skip_queue true (TAB drücken nach Commandeingabe) wird es vorne eingereiht und sofort abgespielt (überspringt das momentane Lied)

/// Plays a song in your current voice channel
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Spielt ein Lied im momentanen Sprachkanal ab"),
    required_bot_permissions = "VIEW_CHANNEL | CONNECT | SPEAK"
)]
pub async fn play(
    ctx: CommandContext<'_>,
    #[description = "YouTube search or direct link to all platforms supported by yt-dlp"]
    #[description_localized(
        "de",
        "YouTube-Suche oder Direktlink zu allen von yt-dlp unterstützten Platformen"
    )]
    #[autocomplete = "autocomplete_yt_video_search"]
    source: String,
    #[description = "Whether the queue should be skipped"]
    #[description_localized("de", "Ob die Warteschlange übersprungen werden soll")]
    skip_queue: Option<bool>,
) -> Result<(), CommandError> {
    // ======== Join the right voice channel or return ========

    // Get user's current voice channel
    let (user_guild, user_channel) = get_author_voice_state(ctx);

    // Return if user not in a voice channel
    let connect_to = user_channel.ok_or(UserNotInVoice)?;

    let songbird = songbird::get(ctx.serenity_context())
        .await
        .ok_or(SongbirdNotFound)?;

    // Make sure the bot is in the right channel
    let call = join_voice(songbird, user_guild, connect_to).await?;

    // ======== Play track ========

    let metadata = enqueue_track(ctx, call.clone(), &source).await?;

    // skip_queue -> Move to the front and skip current track
    if skip_queue.is_some_and(|v| v) {
        let call = call.lock().await;
        let queue = call.queue();

        if queue.len() > 1 {
            queue.modify_queue(|raw_queue| {
                let new = raw_queue.pop_back().unwrap();
                raw_queue.insert(1, new);
                raw_queue.front().unwrap().stop().unwrap();
            });
        }

        let response_details = format!(
            "`{}` wird jetzt in {} abgespielt",
            metadata.title,
            connect_to.to_channel(ctx).await?.mention()
        );
        _ = respond_success(&ctx, "Track Found", response_details, false).await?;
    } else {
        let response_details = format!(
            "`{}` zur Warteschlange für {} hinzugefügt",
            metadata.title,
            connect_to.to_channel(ctx).await?.mention()
        );
        _ = respond_success(&ctx, "Track Found", response_details, false).await?;
    }

    Ok(())
}

async fn autocomplete_yt_playlist_search(
    ctx: CommandContext<'_>,
    partial: &str,
) -> Vec<AutocompleteChoice> {
    if partial.len() < 3 {
        // Discord doesn't like 0-length options
        return vec![AutocompleteChoice::new(
            "Tippe weiter, um Suchvorschläge zu erhalten",
            partial,
        )];
    }

    let youtube_client = get_youtube_client(ctx.serenity_context()).await;

    // YouTube URL
    if let Some(id) = get_yt_id_from_url(partial).playlist_id {
        return match youtube_client.get_playlist(&id).await {
            Ok(video) => vec![AutocompleteChoice::new(video.title, partial)],
            Err(e) => {
                error!("YT playlist lookup for id {} failed: {:?}", id, e);
                vec![AutocompleteChoice::new(partial, partial)]
            }
        };
    }

    // Random text -> search
    match youtube_client
        .search(partial, YtSearchFilter::Playlists, 5)
        .await
    {
        Ok(results) => results
            .into_iter()
            .map(|playlist| {
                AutocompleteChoice::new(&playlist.title, playlist.get_yt_url().as_str())
            })
            .collect(),
        Err(e) => {
            error!("YT search failed: {:?}", e);
            vec![AutocompleteChoice::new(partial, partial)]
        }
    }
}

/// Loads a whole YouTube playlist into the queue
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Lädt eine ganze Youtube-Playlist in die Warteschlange"),
    required_bot_permissions = "VIEW_CHANNEL | CONNECT | SPEAK"
)]
pub async fn playlist(
    ctx: CommandContext<'_>,
    #[description = "YouTube search or direct link to a YouTube playlist"]
    #[description_localized("de", "Youtube-Suche oder Direktlink zu einer YouTube Playlist")]
    #[autocomplete = "autocomplete_yt_playlist_search"]
    source: String,
    #[description = "Whether the tracks should be added in a randomized order"]
    #[description_localized(
        "de",
        "Ob die Lieder in einer zufälligen Reihenfolge hinzugefügt werden sollen"
    )]
    shuffle: Option<bool>,
) -> Result<(), CommandError> {
    // ======== Join the right voice channel or return ========

    // Get user's current voice channel
    let (user_guild, user_channel) = get_author_voice_state(ctx);

    // Return if user not in a voice channel
    let connect_to = user_channel.ok_or(UserNotInVoice)?;

    let songbird = songbird::get(ctx.serenity_context())
        .await
        .ok_or(SongbirdNotFound)?;

    // Make sure the bot is in the right channel
    let call = join_voice(songbird, user_guild, connect_to).await?;

    // ======== Play track ========

    let youtube_client = get_youtube_client(ctx.serenity_context()).await;

    // Get playlist id
    let playlist_id = match get_yt_id_from_url(&source).playlist_id {
        Some(id) => id,
        None => match youtube_client
            .search(&source, YtSearchFilter::Playlists, 1)
            .await
            .ok()
            .and_then(|mut vec| vec.pop().map(|r| r.id))
        {
            Some(YtResourceId::Playlist(id)) => id,
            _ => panic!(),
        },
    };

    let mut playlist = youtube_client.get_playlist(&playlist_id).await.unwrap();
    if shuffle.is_some_and(|s| s) {
        playlist.videos.shuffle(&mut rand::rng());
    }

    call.lock().await.queue().stop();

    //TODO: Send playlist requests in chunks
    for video in playlist.videos {
        enqueue_track(ctx, call.clone(), video.get_yt_url().as_str()).await?;
    }

    let response_details = format!(
        "`{}` wird jetzt in {} abgespielt",
        playlist.title,
        connect_to.to_channel(ctx).await?.mention()
    );
    _ = respond_success(&ctx, "Track Found", response_details, false).await?;

    Ok(())
}

/// Shows information about the currently playing track
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Zeigt informationen über den aktuellen Track")
)]
pub async fn now_playing(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let (_channel_id, call) = get_call(ctx).await?;
    let call = call.lock().await;

    let queue = call.queue();
    let track = queue.current().ok_or(QueueEmpty)?;
    let metadata = track.data::<TrackMetadata>();
    let playback_info = track.get_info().await.unwrap();

    fn format_duration(duration: Duration) -> String {
        let mut secs = duration.as_secs();
        let hours = secs / 3600;
        secs -= hours * 3600;
        let mins = secs / 60;
        secs -= mins * 60;

        let hours_str = if hours != 0 {
            format!("{:02}:", hours)
        } else {
            "".to_owned()
        };
        format!("{}{:02}:{:02}", hours_str, mins, secs)
    }

    let response_details = format!(
        "`Titel`: {}\n`Autor`: {}\n`Quelle`: {}\n`Angefordert von`: {}\n`Position`: {}/{}\n`Loop`: {}",
        metadata.title,
        metadata.author,
        metadata.source_url,
        metadata.requested_by.expect("Request data always present").mention(),
        format_duration(playback_info.position),
        format_duration(metadata.duration),
        if playback_info.loops != LoopState::Finite(0) {
            "aktiviert".to_owned()
        } else {
            "deaktiviert".to_owned()
        }
    );

    _ = respond_success(&ctx, "Now playing", response_details, true).await?;

    Ok(())
}

/// Shows the current queue
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Zeigt die aktuelle Warteschlange")
)]
pub async fn queue(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let (_, call) = get_call(ctx).await?;
    let call = call.lock().await;

    let queue = call.queue();
    if queue.is_empty() {
        _ = respond_success(&ctx, "Queue", "Die Warteschlange ist leer", true).await?;
        return Ok(());
    };

    let track_list = join_all(queue.current_queue().into_iter().enumerate().map(
        |(i, t)| async move {
            let meta = t.data::<TrackMetadata>();
            let icon = if t.get_info().await.unwrap().loops != LoopState::Finite(0) {
                ":repeat:"
            } else {
                ""
            };
            format!("`{}` {icon} [{}]({})", i + 1, meta.title, meta.source_url)
        },
    ))
    .await
    .join("\n");

    _ = respond_success(&ctx, "Queue", track_list, true).await?;

    Ok(())
}

/// Loops the current track until loop is deactivated again, or the track is skipped
#[poise::command(
    rename = "loop",
    slash_command,
    guild_only,
    description_localized(
        "de",
        "Wiederholt das aktuelle Lied bis loop wieder deaktiviert oder es übersprungen wird"
    )
)]
pub async fn loop_command(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let (channel_id, call) = get_call(ctx).await?;

    let current_track = call.lock().await.queue().current().ok_or(QueueEmpty)?;

    let was_looping = current_track.get_info().await.unwrap().loops != LoopState::Finite(0);

    if was_looping {
        _ = current_track.disable_loop()
    } else {
        _ = current_track.enable_loop()
    }

    let response_details = format!(
        "Wiederholung für `{}` in {} {}",
        current_track.data::<TrackMetadata>().title,
        channel_id.to_channel(ctx).await?.mention(),
        if was_looping {
            "deaktiviert"
        } else {
            "aktiviert"
        }
    );

    _ = respond_success(&ctx, "Loop", response_details, false).await?;

    Ok(())
}

/// Skips the currently playing track
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Überspringt das aktuelle Lied")
)]
pub async fn skip(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let (channel_id, call) = get_call(ctx).await?;
    let call = call.lock().await;

    let queue = call.queue();
    let skipped = queue.current().ok_or(QueueEmpty)?;
    _ = queue.skip();

    let response_details = format!(
        "`{}` in Kanal {} übersprungen{}",
        &skipped.data::<TrackMetadata>().author,
        channel_id.to_channel(ctx).await?.mention(),
        match queue.current() {
            Some(t) => format!(
                "\n`{}` wird jetzt abgespielt",
                t.data::<TrackMetadata>().title
            ),
            None => "".to_owned(),
        }
    );

    _ = respond_success(&ctx, "Skipped", response_details, false).await?;

    Ok(())
}

/// Stops playback and clears the queue
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Stoppt die aktive Wiedergabe und leert die Warteschlange")
)]
pub async fn stop(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let (channel_id, call) = get_call(ctx).await?;
    let call = call.lock().await;

    let queue = call.queue();
    if queue.is_empty() {
        return Err(QueueEmpty);
    };
    queue.stop();

    let response_details = format!(
        "Wiedergabe in Kanal {} gestoppt und Warteliste geleert",
        channel_id.to_channel(ctx).await?.mention()
    );

    _ = respond_success(&ctx, "Stopped", response_details, false).await?;

    Ok(())
}

/// Leaves the current channel
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Verlässt den aktuellen Channel")
)]
pub async fn leave(ctx: CommandContext<'_>) -> Result<(), CommandError> {
    let (channel_id, call) = get_call(ctx).await?;
    let mut call = call.lock().await;

    call.queue().stop();
    call.stop();
    call.leave().await.map_err(|_| LeaveVoice)?;

    let response_details = format!("{} verlassen", channel_id.to_channel(ctx).await?.mention());
    _ = respond_success(&ctx, "Left", response_details, false).await?;

    Ok(())
}
