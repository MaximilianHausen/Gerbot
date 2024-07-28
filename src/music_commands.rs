use crate::{Context, Error, GuildData};
use poise::{CreateReply, ReplyHandle};
use reqwest::Client as HttpClient;
use serenity::builder::CreateEmbed;
use serenity::model::Colour;
use songbird::input::YoutubeDl;

const RESPONSE_COLOUR: Colour = Colour::BLURPLE;
const ERROR_COLOUR: Colour = Colour::RED;

async fn get_http_client(ctx: &serenity::client::Context) -> HttpClient {
    let data = ctx.data.read().await;
    data.get::<crate::HttpKey>()
        .cloned()
        .expect("Guaranteed to exist in the typemap.")
}

fn map_err<T>(send_result: Result<T, serenity::Error>) -> Result<T, Error> {
    send_result.map_err(|e| Box::new(e) as Error)
}

async fn respond_err<'a>(
    ctx: &'a Context<'a>,
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

/// Infos about the available commands
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    description_localized("de", "Infos zu den verfügbaren Commands")
)]
pub async fn help(ctx: Context<'_>) -> Result<(), Error> {
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

//TODO: Help command text
//Spielt ein Lied im momentanen Channel ab. Als Quelle geht ein Suchbegriff für Youtube oder ein direkter Link zu allen von yt-dlp unterstützten [Platformen](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md). Standartmäßig wird das Lied hinten in die Warteschlange eingereiht. Mit skip_queue true (TAB drücken nach Commandeingabe) wird es vorne eingereiht und sofort abgespielt (überspringt das momentane Lied)

/// Plays a song in your current voice channel
#[poise::command(
    slash_command,
    guild_only,
    description_localized("de", "Spielt ein Lied im momentanen Voice Channel ab")
)]
pub async fn play(
    ctx: Context<'_>,
    #[description = "Youtube search or direct link to all platforms supported by yt-dlp"]
    #[description_localized(
        "de",
        "Youtube-Suche oder Direktlink zu allen von yt-dlp unterstützten Platformen"
    )]
    source: String,
    #[description = "If the queue should be skipped"]
    #[description_localized("de", "Ob die Warteschlange übersprungen werden soll")]
    skip_queue: Option<bool>,
) -> Result<(), Error> {
    // ======== Join the right voice channel or return ========

    // Get user's current voice channel
    let (guild_id, channel_id) = {
        let guild = ctx.guild().unwrap();
        let channel_id = guild
            .voice_states
            .get(&ctx.author().id)
            .and_then(|voice_state| voice_state.channel_id);

        (guild.id, channel_id)
    };

    // Return in user not in a voice channel
    let connect_to = match channel_id {
        Some(channel) => channel,
        None => {
            _ = respond_err(&ctx, "Du bist nicht in einem Sprachkanal in diesem Server").await?;
            return Ok(());
        }
    };

    let songbird = songbird::get(ctx.serenity_context()).await.unwrap();

    // Make sure the bot is in the right channel
    let call = if let Some(call) = songbird.get(guild_id) {
        let current_channel = call.lock().await.current_channel();
        if current_channel.is_some_and(|c| c != connect_to.into()) {
            _ = respond_err(
                &ctx,
                "Der Bot wird bereits in einem anderen Sprachkanal verwendet",
            )
            .await?;
            return Ok(());
        }
        call
    } else {
        // Bot not in a channel -> join
        match songbird.join(guild_id, connect_to).await {
            Ok(c) => c,
            Err(_) => {
                _ = respond_err(&ctx, "Der Bot konnte deinem Sprachkanal nicht beitreten").await?;
                //TODO: Maybe don't return ok on join fail
                return Ok(());
            }
        }
    };

    // ======== Play song ========

    let http_client = get_http_client(ctx.serenity_context()).await;
    let do_search = !source.starts_with("http");

    // yt-dlp lookup
    let src = if do_search {
        YoutubeDl::new_search(http_client, source)
    } else {
        YoutubeDl::new(http_client, source)
    };

    // Play and save track
    {
        let track_handle = call.lock().await.play_input(src.clone().into());
        let mut guild_data_map = ctx.data().per_guild_data.lock().await;

        if let Some(guild_data) = guild_data_map.get_mut(&guild_id) {
            guild_data.current_track = track_handle;
        } else {
            guild_data_map.insert(
                guild_id,
                GuildData {
                    current_track: track_handle,
                },
            );
        }
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
