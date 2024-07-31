use std::collections::HashMap;

use lazy_static::lazy_static;
use log::info;
use reqwest::{Client as HttpClient, StatusCode, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::RwLock;

lazy_static! {
    static ref RATE_LIMITED_DAY: RwLock<Option<i32>> = RwLock::new(None);
}

// ======== Error type ========

#[derive(Error, Debug)]
pub enum YtSearchError {
    #[error("Request error")]
    Request(#[from] reqwest::Error),
    #[error("Api error")]
    Api,
}

// ======== Json model ========

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtSearchListModel {
    etag: String,
    next_page_token: Option<String>,
    prev_page_token: Option<String>,
    region_code: String,
    page_info: YtSearchListPageInfo,
    items: Vec<YtSearchResultModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtSearchListPageInfo {
    total_results: u32,
    results_per_page: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtSearchResultModel {
    etag: String,
    id: YtSearchResultId,
    snippet: YtSearchResultSnippet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YtSearchResultId {
    kind: String,
    video_id: Option<String>,
    channel_id: Option<String>,
    playlist_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum YtThumbnailSize {
    Default,
    Medium,
    High,
    Standart,
    Maxres,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YtThumbnailInfo {
    pub url: Url,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum YtLiveStatus {
    None,
    Live,
    Upcoming,
}

// ======== Compact result struct ========

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtSearchResult {
    pub id: String,
    pub title: String,
    pub description: String,
    pub published_at: OffsetDateTime,
    pub channel_id: String,
    pub channel_title: String,
    pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
    #[serde(rename = "liveBroadcastContent")]
    pub live_status: YtLiveStatus,
}

impl YtSearchResult {
    pub fn get_yt_url(&self) -> Url {
        Url::parse(&format!("https://www.youtube.com/watch?v={}", self.id)).unwrap()
    }
}

impl From<YtSearchResultModel> for YtSearchResult {
    fn from(value: YtSearchResultModel) -> Self {
        YtSearchResult {
            id: value.id.video_id.unwrap(),
            title: value.snippet.title,
            description: value.snippet.description,
            published_at: value.snippet.published_at,
            channel_id: value.snippet.channel_id,
            channel_title: value.snippet.channel_title,
            thumbnails: value.snippet.thumbnails,
            live_status: value.snippet.live_status,
        }
    }
}

/// Low latency YouTube search request for videos
pub async fn yt_search(
    query: &str,
    n_results: u8,
    http_client: HttpClient,
    yt_api_key: Option<&str>
) -> Result<Vec<YtSearchResult>, YtSearchError> {
    {
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

    let url = if RATE_LIMITED_DAY.read().await.is_none() && yt_api_key.is_some() {
        format!("https://www.googleapis.com/youtube/v3/search?key={}&part=snippet&type=video&q={query}&maxResults={n_results}", yt_api_key.as_ref().unwrap())
    } else {
        format!("https://yt.lemnoslife.com/noKey/search?part=snippet&type=video&q={query}&maxResults={n_results}")
    };

    let response = http_client.get(url).send().await?;

    match response.status() {
        StatusCode::OK => {
            let parsed_response = response.json::<YtSearchListModel>().await?;
            Ok(parsed_response
                .items
                .into_iter()
                .map(|i| i.into())
                .collect())
        }
        StatusCode::FORBIDDEN => {
            //TODO: Parse yt api errors for more appropriate handling
            *RATE_LIMITED_DAY.write().await =
                Some(OffsetDateTime::now_utc().date().to_julian_day());
            info!("Encountered rate limit from YouTube API. Switching to fallback proxy");
            Err(YtSearchError::Api)
        }
        _ => Err(YtSearchError::Api),
    }
}
