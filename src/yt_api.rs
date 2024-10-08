#![allow(dead_code)]

use lazy_static::lazy_static;
use log::info;
use reqwest::{Client as HttpClient, Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::RwLock;

lazy_static! {
    static ref RATE_LIMITED_DAY: RwLock<Option<i32>> = RwLock::new(None);
}

// =============================
// ======== Json models ========
// =============================

// These models might be missing some minor values

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtListModel<T> {
    etag: String,
    next_page_token: Option<String>,
    prev_page_token: Option<String>,
    page_info: YtListPageInfo,
    items: Vec<T>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtListPageInfo {
    total_results: u32,
    results_per_page: u32,
}

// ======== Search ========

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtSearchResultModel {
    etag: String,
    id: YtSearchResultId,
    snippet: YtSearchResultSnippet,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtSearchResultId {
    kind: String,
    video_id: Option<String>,
    channel_id: Option<String>,
    playlist_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtSearchResultSnippet {
    title: String,
    description: String,
    #[serde(with = "time::serde::iso8601")]
    published_at: OffsetDateTime,
    channel_id: String,
    channel_title: String,
    thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
    #[serde(rename = "liveBroadcastContent")]
    live_status: YtLiveStatus,
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
pub enum YtLiveStatus {
    None,
    Live,
    Upcoming,
}

// ======== Video ========

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtVideoModel {
    etag: String,
    id: String,
    snippet: Option<YtVideoSnippet>,
    content_details: Option<YtVideoContentDetails>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtVideoSnippet {
    #[serde(with = "time::serde::iso8601")]
    published_at: OffsetDateTime,
    channel_id: String,
    title: String,
    description: String,
    thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
    channel_title: String,
    tags: Vec<String>,
    category_id: String,
    #[serde(rename = "liveBroadcastContent")]
    live_status: YtLiveStatus,
    // Missing localization values (defaultLanguage, localized, defaultAudioLanguage)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtVideoContentDetails {
    #[serde(with = "crate::serde::iso_duration")]
    duration: Duration,
    dimension: YtVideoDimension,
    definition: YtVideoDefinition,
    /// "true" or "false" (why not boolean?)
    caption: String,
    licensed_content: bool,
    region_restriction: Option<YtVideoRegionRestriction>,
    // content_rating not here because it is a really complicated type
    projection: YtVideoProjection,
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
    allowed: Vec<String>,
    blocked: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum YtVideoProjection {
    #[serde(rename = "rectangular")]
    Rectangular,
    #[serde(rename = "360")]
    _360,
}

// ===========================
// ======== Functions ========
// ===========================

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtVideo {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Only available from video details, not from search
    pub duration: Option<Duration>,
    pub published_at: OffsetDateTime,
    pub channel_id: String,
    pub channel_title: String,
    pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
    #[serde(rename = "liveBroadcastContent")]
    pub live_status: YtLiveStatus,
}

impl YtVideo {
    pub fn get_yt_url(&self) -> Url {
        Url::parse(&format!("https://www.youtube.com/watch?v={}", self.id)).unwrap()
    }
}

impl From<YtSearchResultModel> for YtVideo {
    fn from(value: YtSearchResultModel) -> Self {
        YtVideo {
            id: value.id.video_id.unwrap(),
            title: value.snippet.title,
            description: value.snippet.description,
            duration: None,
            published_at: value.snippet.published_at,
            channel_id: value.snippet.channel_id,
            channel_title: value.snippet.channel_title,
            thumbnails: value.snippet.thumbnails,
            live_status: value.snippet.live_status,
        }
    }
}

impl From<YtVideoModel> for YtVideo {
    fn from(value: YtVideoModel) -> Self {
        let snippet = value.snippet.unwrap();
        let content_details = value.content_details.unwrap();

        YtVideo {
            id: value.id,
            title: snippet.title,
            description: snippet.description,
            duration: Some(content_details.duration),
            published_at: snippet.published_at,
            channel_id: snippet.channel_id,
            channel_title: snippet.channel_title,
            thumbnails: snippet.thumbnails,
            live_status: snippet.live_status,
        }
    }
}

#[derive(Error, Debug)]
pub enum YtApiError {
    #[error("Request error")]
    Request(#[from] reqwest::Error),
    #[error("Youtube API error")]
    Api,
    #[error("The provided video id does not exist")]
    InvalidVideoId,
}

/// Low latency YouTube search request for videos
pub async fn yt_search(
    query: &str,
    n_results: u8,
    http_client: HttpClient,
    yt_api_key: Option<&str>,
) -> Result<Vec<YtVideo>, YtApiError> {
    try_clear_ratelimit().await;

    let url = if RATE_LIMITED_DAY.read().await.is_none() && yt_api_key.is_some() {
        format!("https://www.googleapis.com/youtube/v3/search?part=snippet&type=video&q={query}&maxResults={n_results}&key={}", yt_api_key.as_ref().unwrap())
    } else {
        format!("https://yt.lemnoslife.com/noKey/search?part=snippet&type=video&q={query}&maxResults={n_results}")
    };

    let response = http_client.get(url).send().await?;

    process_api_response::<YtListModel<YtSearchResultModel>>(response)
        .await
        .map(|ok| ok.items.into_iter().map(|i| i.into()).collect())
}

/// Query details for a YouTube video
pub async fn yt_video_details(
    id: &str,
    http_client: HttpClient,
    yt_api_key: Option<&str>,
) -> Result<YtVideo, YtApiError> {
    try_clear_ratelimit().await;

    let url = if RATE_LIMITED_DAY.read().await.is_none() && yt_api_key.is_some() {
        format!("https://www.googleapis.com/youtube/v3/videos?part=contentDetails,snippet&id={id}&key={}", yt_api_key.as_ref().unwrap())
    } else {
        format!("https://yt.lemnoslife.com/noKey/videos?part=contentDetails,snippet&id={id}")
    };

    let response = http_client.get(url).send().await?;

    process_api_response::<YtListModel<YtVideoModel>>(response)
        .await
        .map(|list| {
            list.items
                .into_iter()
                .next()
                .map(|i| i.into())
                .ok_or(YtApiError::InvalidVideoId)
        })?
}

async fn try_clear_ratelimit() {
    // Clear rate limit on the next day
    let rate_limit_lock = RATE_LIMITED_DAY.read().await;

    if let Some(day) = *rate_limit_lock {
        if day < OffsetDateTime::now_utc().to_julian_day() {
            // drop read before acquiring write lock
            drop(rate_limit_lock);
            *RATE_LIMITED_DAY.write().await = None;
            info!("Cleared rate limit for official YouTube API");
        }
    }
}

async fn process_api_response<T: DeserializeOwned>(response: Response) -> Result<T, YtApiError> {
    match response.status() {
        StatusCode::OK => {
            let parsed_response = response.json::<T>().await?;
            Ok(parsed_response)
        }
        StatusCode::FORBIDDEN => {
            //TODO: Parse yt api errors for more appropriate handling
            *RATE_LIMITED_DAY.write().await =
                Some(OffsetDateTime::now_utc().date().to_julian_day());
            info!("Encountered rate limit from YouTube API. Switching to fallback proxy");
            Err(YtApiError::Api)
        }
        _ => Err(YtApiError::Api),
    }
}
