#![allow(dead_code)]

use crate::youtube::YtResourceId::{Channel, Playlist, Video};
use crate::youtube::{YtApiError, YtPlaylist, YtResource, YtSearchFilter, YtVideo};
use log::info;
use reqwest::{Client as HttpClient, Response, StatusCode};
use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use tokio::try_join;

// =============================
// ======== Json models ========
// =============================

pub(super) mod models {
    use reqwest::Url;
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::time::Duration;
    use time::OffsetDateTime;

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtList<T> {
        pub etag: String,
        pub next_page_token: Option<String>,
        pub prev_page_token: Option<String>,
        pub page_info: YtListPageInfo,
        pub items: Vec<T>,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtListPageInfo {
        pub total_results: u32,
        pub results_per_page: u32,
    }

    // ======== Search ======== (https://developers.google.com/youtube/v3/docs/search#resource-representation)

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtSearchResult {
        pub etag: String,
        pub id: YtSearchResultId,
        pub snippet: YtSearchResultSnippet,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtSearchResultId {
        pub kind: String,
        pub video_id: Option<String>,
        pub channel_id: Option<String>,
        pub playlist_id: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtSearchResultSnippet {
        #[serde(with = "time::serde::iso8601")]
        pub published_at: OffsetDateTime,
        pub channel_id: String,
        pub title: String,
        pub description: String,
        pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
        pub channel_title: String,
        pub live_broadcast_content: YtLiveBroadcastContent,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum YtThumbnailSize {
        Default,
        Medium,
        High,
        Standard,
        Maxres,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct YtThumbnailInfo {
        pub url: Url,
        pub width: u32,
        pub height: u32,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum YtLiveBroadcastContent {
        None,
        Live,
        Upcoming,
    }

    // ======== Video ======== (https://developers.google.com/youtube/v3/docs/videos#resource-representation)

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtVideo {
        // Lots of irrelevant extra data skipped here
        pub etag: String,
        pub id: String,
        pub snippet: YtVideoSnippet, // Optional, but there is no point in not requesting it
        pub content_details: YtVideoContentDetails, // Optional, but there is no point in not requesting it
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtVideoSnippet {
        #[serde(with = "time::serde::iso8601")]
        pub published_at: OffsetDateTime,
        pub channel_id: String,
        pub title: String,
        pub description: String,
        pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
        pub channel_title: String,
        #[serde(default)]
        pub tags: Vec<String>,
        pub category_id: String,
        pub live_broadcast_content: YtLiveBroadcastContent,
        // Missing localization values (defaultLanguage, localized, defaultAudioLanguage)
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtVideoContentDetails {
        #[serde(with = "crate::serde::iso_duration")]
        pub duration: Duration,
        pub dimension: YtVideoDimension,
        pub definition: YtVideoDefinition,
        #[serde(with = "crate::serde::bool_string")]
        pub caption: bool,
        pub licensed_content: bool,
        pub region_restriction: Option<YtVideoRegionRestriction>,
        // content_rating not here because it is a really complicated type
        pub projection: YtVideoProjection,
        // has_custom_thumbnail not here because it is only visible to the uploader
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum YtVideoDimension {
        #[serde(rename = "2d")]
        _2D,
        #[serde(rename = "3d")]
        _3D,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum YtVideoDefinition {
        SD,
        HD,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtVideoRegionRestriction {
        #[serde(default)]
        pub allowed: Vec<String>,
        #[serde(default)]
        pub blocked: Vec<String>,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum YtVideoProjection {
        #[serde(rename = "rectangular")]
        Rectangular,
        #[serde(rename = "360")]
        _360,
    }

    // ======== Playlist ======== (https://developers.google.com/youtube/v3/docs/playlistItems#resource-representation)

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtPlaylist {
        // Some stuff skipped here
        pub etag: String,
        pub id: String,
        pub snippet: YtPlaylistSnippet, // Optional, but there is no point in not requesting it
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtPlaylistSnippet {
        #[serde(with = "time::serde::iso8601")]
        pub published_at: OffsetDateTime,
        pub channel_id: String,
        pub title: String,
        pub description: String,
        pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
        pub channel_title: String,
        // Missing localization values (defaultLanguage, localized)
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtPlaylistItem {
        pub etag: String,
        pub id: String,
        pub snippet: YtPlaylistItemSnippet,
        pub content_details: YtPlaylistItemContentDetails,
        // 'status' with privacy status skipped
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtPlaylistItemSnippet {
        #[serde(with = "time::serde::iso8601")]
        pub published_at: OffsetDateTime,
        pub channel_id: String,
        pub title: String,
        pub description: String,
        pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
        pub channel_title: String,
        pub video_owner_channel_title: String,
        pub video_owner_channel_id: String,
        pub playlist_id: String,
        pub position: u32,
        pub resource_id: YtPlaylistItemResourceId,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtPlaylistItemResourceId {
        pub kind: String,
        pub video_id: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct YtPlaylistItemContentDetails {
        pub video_id: String,
        pub note: Option<String>,
        #[serde(with = "time::serde::iso8601")]
        pub video_published_at: OffsetDateTime,
    }
}

impl From<models::YtSearchResult> for YtResource {
    fn from(value: models::YtSearchResult) -> Self {
        Self {
            id: match value.id.kind.as_str() {
                "youtube#video" => Video(value.id.video_id.unwrap()),
                "youtube#playlist" => Playlist(value.id.playlist_id.unwrap()),
                "youtube#channel" => Channel(value.id.channel_id.unwrap()),
                _ => unreachable!(),
            },
            title: value.snippet.title,
            description: value.snippet.description,
            published_at: value.snippet.published_at,
            channel_id: value.snippet.channel_id,
            channel_title: value.snippet.channel_title,
            thumbnails: value.snippet.thumbnails,
        }
    }
}

impl From<models::YtPlaylistItem> for YtResource {
    fn from(value: models::YtPlaylistItem) -> Self {
        Self {
            id: Video(value.content_details.video_id),
            title: value.snippet.title,
            description: value.snippet.description,
            published_at: value.content_details.video_published_at,
            channel_id: value.snippet.video_owner_channel_id,
            channel_title: value.snippet.video_owner_channel_title,
            thumbnails: value.snippet.thumbnails,
        }
    }
}

impl From<models::YtVideo> for YtVideo {
    fn from(value: models::YtVideo) -> Self {
        Self {
            id: value.id,
            title: value.snippet.title,
            description: value.snippet.description,
            duration: value.content_details.duration,
            published_at: value.snippet.published_at,
            channel_id: value.snippet.channel_id,
            channel_title: value.snippet.channel_title,
            thumbnails: value.snippet.thumbnails,
            live_status: value.snippet.live_broadcast_content,
        }
    }
}

impl From<models::YtPlaylist> for YtPlaylist {
    fn from(value: models::YtPlaylist) -> Self {
        Self {
            id: value.id,
            title: value.snippet.title,
            description: value.snippet.description,
            published_at: value.snippet.published_at,
            channel_id: value.snippet.channel_id,
            channel_title: value.snippet.channel_title,
            thumbnails: value.snippet.thumbnails,
            videos: vec![],
        }
    }
}

// ===========================
// ======== Functions ========
// ===========================

#[derive(Debug)]
pub struct YtApiClient {
    http_client: HttpClient,
    yt_api_key: String,
    rate_limited_day: RwLock<Option<i32>>,
}

impl YtApiClient {
    pub fn new(http_client: HttpClient, yt_api_key: String) -> Self {
        Self {
            http_client,
            yt_api_key,
            rate_limited_day: RwLock::new(None),
        }
    }

    //TODO: Implement etags for search https://developers.google.com/youtube/v3/getting-started#etags
    pub async fn search(
        &self,
        query: &str,
        filter: YtSearchFilter,
        n_results: u8,
    ) -> Result<Vec<YtResource>, YtApiError> {
        let type_str = match filter {
            YtSearchFilter::Videos => "video",
            YtSearchFilter::Playlists => "playlist",
            YtSearchFilter::Channels => "channel",
            YtSearchFilter::Any => "channel,playlist,video",
        };
        let url = format!("https://www.googleapis.com/youtube/v3/search?part=snippet&type={type_str}&q={query}&maxResults={n_results}&key={}", self.yt_api_key);

        let response = self.http_client.get(url).send().await?;

        self.process_api_response::<models::YtList<models::YtSearchResult>>(response)
            .await
            .map(|list| list.items.into_iter().map(YtResource::from).collect())
    }

    pub async fn get_video(&self, id: &str) -> Result<YtVideo, YtApiError> {
        let url = format!("https://www.googleapis.com/youtube/v3/videos?part=contentDetails,snippet&id={id}&key={}", self.yt_api_key);

        let response = self.http_client.get(url).send().await?;

        self.process_api_response::<models::YtList<models::YtVideo>>(response)
            .await
            .and_then(|list| {
                list.items
                    .into_iter()
                    .next()
                    .map(YtVideo::from)
                    .ok_or(YtApiError::InvalidId)
            })
    }

    pub async fn get_playlist(&self, id: &str) -> Result<YtPlaylist, YtApiError> {
        let meta_url = format!(
            "https://www.googleapis.com/youtube/v3/playlists?part=snippet&id={id}&key={}",
            self.yt_api_key
        );
        let items_url = format!("https://www.googleapis.com/youtube/v3/playlistItems?part=snippet,contentDetails&playlistId={id}&maxResults=50&key={}", self.yt_api_key);

        let meta_future = self.http_client.get(meta_url).send();
        let items_future = self.http_client.get(items_url).send();
        let (meta_response, items_response) = try_join!(meta_future, items_future)?;

        let mut playlist = self
            .process_api_response::<models::YtList<models::YtPlaylist>>(meta_response)
            .await
            .and_then(|list| {
                list.items
                    .into_iter()
                    .next()
                    .map(YtPlaylist::from)
                    .ok_or(YtApiError::InvalidId)
            })?;

        let items = self
            .process_api_response::<models::YtList<models::YtPlaylistItem>>(items_response)
            .await
            .map(|list| list.items.into_iter().map(YtResource::from).collect())?;

        playlist.videos = items;

        Ok(playlist)
    }

    pub async fn is_ratelimited(&self) -> bool {
        let rate_limit_lock = self.rate_limited_day.read().await;

        if let Some(day) = *rate_limit_lock {
            // Clear rate limit on the next day
            if day < OffsetDateTime::now_utc().to_julian_day() {
                // drop read before acquiring write lock
                drop(rate_limit_lock);
                *self.rate_limited_day.write().await = None;
                info!("Cleared rate limit for official YouTube API");
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    async fn process_api_response<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, YtApiError> {
        match response.status() {
            StatusCode::OK => {
                let parsed_response = response.json::<T>().await?;
                Ok(parsed_response)
            }
            StatusCode::FORBIDDEN => {
                //TODO: Parse yt api errors for more appropriate handling
                *self.rate_limited_day.write().await =
                    Some(OffsetDateTime::now_utc().date().to_julian_day());
                info!("Encountered rate limit from YouTube API. Disabling for the day.");
                Err(YtApiError::QuotaExceeded)
            }
            _ => Err(YtApiError::Api),
        }
    }
}
