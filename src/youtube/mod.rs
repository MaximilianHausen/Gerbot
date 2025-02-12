#![allow(dead_code)]

use crate::youtube::YtResourceId::{Channel, Playlist, Video};
use reqwest::{Client as HttpClient, Url};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use log::debug;
use thiserror::Error;
use time::OffsetDateTime;

mod yt_api;

use crate::youtube::yt_api::YtApiClient;
pub use yt_api::models::YtLiveBroadcastContent;
pub use yt_api::models::YtThumbnailInfo;
pub use yt_api::models::YtThumbnailSize;

// =================
// ==== Structs ====
// =================

#[derive(Clone, Debug)]
pub enum YtResourceId {
    Video(String),
    Playlist(String),
    Channel(String),
}

#[derive(Clone, Debug)]
pub struct YtResource {
    pub id: YtResourceId,
    pub title: String,
    pub description: String,
    pub published_at: OffsetDateTime,
    pub channel_id: String,
    pub channel_title: String,
    pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
}

impl YtResource {
    pub fn get_yt_url(&self) -> Url {
        match &self.id {
            Video(id) => Url::parse(&format!("https://www.youtube.com/watch?v={id}")).unwrap(),
            Playlist(id) => {
                Url::parse(&format!("https://www.youtube.com/playlist?list={id}")).unwrap()
            }
            Channel(id) => Url::parse(&format!("https://www.youtube.com/channel/{id}")).unwrap(),
        }
    }
}

impl From<YtVideo> for YtResource {
    fn from(value: YtVideo) -> Self {
        Self {
            id: Video(value.id),
            title: value.title,
            description: value.description,
            published_at: value.published_at,
            channel_id: value.channel_id,
            channel_title: value.channel_title,
            thumbnails: value.thumbnails,
        }
    }
}

impl From<YtPlaylist> for YtResource {
    fn from(value: YtPlaylist) -> Self {
        Self {
            id: Playlist(value.id),
            title: value.title,
            description: value.description,
            published_at: value.published_at,
            channel_id: value.channel_id,
            channel_title: value.channel_title,
            thumbnails: value.thumbnails,
        }
    }
}

#[derive(Clone, Debug)]
pub struct YtVideo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub duration: Duration,
    pub published_at: OffsetDateTime,
    pub channel_id: String,
    pub channel_title: String,
    pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
    pub live_status: YtLiveBroadcastContent,
}

impl YtVideo {
    pub fn get_yt_url(&self) -> Url {
        Url::parse(&format!("https://www.youtube.com/watch?v={}", self.id)).unwrap()
    }
}

#[derive(Clone, Debug)]
pub struct YtPlaylist {
    pub id: String,
    pub title: String,
    pub description: String,
    pub published_at: OffsetDateTime,
    pub channel_id: String,
    pub channel_title: String,
    pub thumbnails: HashMap<YtThumbnailSize, YtThumbnailInfo>,
    pub videos: Vec<YtResource>,
}

impl YtPlaylist {
    pub fn get_yt_url(&self) -> Url {
        Url::parse(&format!(
            "https://www.youtube.com/playlist?list={}",
            self.id
        ))
        .unwrap()
    }
}

// ================
// ==== Client ====
// ================

#[derive(Error, Debug)]
pub enum YtApiError {
    #[error("Request error")]
    Request(#[from] reqwest::Error),
    #[error("Youtube API error")]
    Api,
    #[error("The provided id does not exist, or is for a different resource type")]
    InvalidId,
    #[error("The youtube api quota for today are used up")]
    QuotaExceeded,
}

pub enum YtSearchFilter {
    Videos,
    Playlists,
    Channels,
    Any,
}

#[derive(Clone, Debug)]
pub struct YoutubeClient {
    pub yt_api_client: Option<Arc<YtApiClient>>,
}

impl YoutubeClient {
    pub fn new(http_client: HttpClient, yt_api_key: Option<String>) -> Self {
        Self {
            yt_api_client: yt_api_key.map(|key| Arc::new(YtApiClient::new(http_client, key))),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        filter: YtSearchFilter,
        n_results: u8,
    ) -> Result<Vec<YtResource>, YtApiError> {
        debug!("Youtube search: {query}");
        match &self.yt_api_client {
            Some(yt_api_client) if !yt_api_client.is_ratelimited().await => {
                yt_api_client.search(query, filter, n_results).await
            }
            _ => {
                //TODO: Implement Invidious as youtube api fallback
                Err(YtApiError::QuotaExceeded)
            }
        }
    }

    pub async fn get_video(&self, id: &str) -> Result<YtVideo, YtApiError> {
        debug!("Youtube video by id: {id}");
        match &self.yt_api_client {
            Some(yt_api_client) if !yt_api_client.is_ratelimited().await => {
                yt_api_client.get_video(id).await
            }
            _ => {
                //TODO: Implement Invidious as youtube api fallback
                Err(YtApiError::QuotaExceeded)
            }
        }
    }

    pub async fn get_playlist(&self, id: &str) -> Result<YtPlaylist, YtApiError> {
        debug!("Youtube playlist by id: {id}");
        match &self.yt_api_client {
            Some(yt_api_client) if !yt_api_client.is_ratelimited().await => {
                yt_api_client.get_playlist(id).await
            }
            _ => {
                //TODO: Implement Invidious as youtube api fallback
                Err(YtApiError::QuotaExceeded)
            }
        }
    }
}
