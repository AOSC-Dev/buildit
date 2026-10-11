use crate::{
    RemoteAddr,
    feed::{EventContent, FEED_TX, FeedEvent},
};
use axum::{
    extract::{ConnectInfo, Query},
    response::{IntoResponse, Sse, sse},
};
use serde::Deserialize;
use std::sync::Arc;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};
use tracing::info;

#[derive(Deserialize)]
pub struct GlobalFeedQuery {
    #[serde(default)]
    filter_pipeline: Option<i32>,
    #[serde(default)]
    filter_job: Option<i32>,
}

pub async fn sse_global_feed_handler(
    Query(query): Query<GlobalFeedQuery>,
    ConnectInfo(addr): ConnectInfo<RemoteAddr>,
) -> impl IntoResponse {
    info!("{:?} connected to global feed", addr);

    let feed_rx = FEED_TX.get().unwrap().subscribe();
    let feed_stream = BroadcastStream::new(feed_rx);
    let sse_stream = feed_stream
        .map(|event| event.unwrap_or_else(|_| Arc::new(FeedEvent::new_lagged())))
        .filter(move |event| filter_feed_events(event, &query))
        .map(serialize_feed_to_sse_event);
    Sse::new(sse_stream).keep_alive(sse::KeepAlive::new())
}

fn serialize_feed_to_sse_event(event: Arc<FeedEvent>) -> Result<sse::Event, axum::Error> {
    sse::Event::default()
        .id(event.id.to_string())
        .json_data(&event.content)
}

fn filter_feed_events(event: &FeedEvent, query: &GlobalFeedQuery) -> bool {
    if matches!(event.content, EventContent::FeedLagged) {
        // always deliver lag notification
        return true;
    }
    if let Some(filter_pipeline) = query.filter_pipeline
        && !event.content.is_related_to_pipeline(filter_pipeline)
    {
        return false;
    }
    if let Some(filter_job) = query.filter_job
        && !event.content.is_related_to_job(filter_job)
    {
        return false;
    }

    true
}
