use crate::youtube::YtVideo;
use reqwest::Url;
use serenity::all::UserId;
use songbird::input::AuxMetadata;
use std::time::Duration;

/// Minimal metadata required by the music commands
pub struct TrackMetadata {
    pub title: String,
    pub author: String,
    pub duration: Duration,
    pub source_url: Url,
    pub requested_by: Option<UserId>,
}

impl Default for TrackMetadata {
    fn default() -> Self {
        Self {
            title: "Unknown".to_string(),
            author: "Unknown".to_string(),
            duration: Duration::default(),
            source_url: Url::parse("https://example.com").unwrap(),
            requested_by: None,
        }
    }
}

impl TrackMetadata {
    pub fn from_with_request(value: impl Into<Self>, requested_by: UserId) -> TrackMetadata {
        TrackMetadata {
            requested_by: Some(requested_by),
            ..value.into()
        }
    }
}

impl From<AuxMetadata> for TrackMetadata {
    fn from(value: AuxMetadata) -> Self {
        TrackMetadata {
            title: value.title.unwrap_or_else(|| "Unknown".to_owned()),
            author: value.artist.unwrap_or_else(|| "Unknown".to_owned()),
            duration: value.duration.unwrap_or_default(),
            source_url: value
                .source_url
                .and_then(|url| Url::parse(&url).ok())
                .unwrap_or(Url::parse("https://example.com").unwrap()),
            requested_by: None,
        }
    }
}

impl From<YtVideo> for TrackMetadata {
    fn from(value: YtVideo) -> Self {
        Self {
            source_url: value.get_yt_url(), // This is first to make the borrow checker happy
            title: value.title,
            author: value.channel_title,
            duration: value.duration,
            requested_by: None,
        }
    }
}
