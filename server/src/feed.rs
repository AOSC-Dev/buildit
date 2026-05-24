use std::sync::{Arc, OnceLock};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::routes::{JobInfoResponse, PipelineInfoResponse};

pub static FEED_TX: OnceLock<broadcast::Sender<Arc<FeedEvent>>> = OnceLock::new();

pub fn deliver_feed_event(content: EventContent) {
    let event = FeedEvent::new(content);
    // The only possible error is "no subscribers", which can be safely ignored.
    _ = FEED_TX.get().unwrap().send(Arc::new(event));
}

pub fn deliver_feed_events<I: IntoIterator<Item = EventContent>>(contents: I) {
    let tx = FEED_TX.get().unwrap();
    for content in contents.into_iter() {
        let event = FeedEvent::new(content);
        if tx.send(Arc::new(event)).is_err() {
            // The only possible error is "no subscribers",
            // in which future events can be skipped
            break;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedEvent {
    pub id: u64,
    pub content: EventContent,
}

impl FeedEvent {
    pub fn new_lagged() -> Self {
        Self::new(EventContent::FeedLagged)
    }

    pub fn new(content: EventContent) -> Self {
        Self {
            id: Self::generate_id(),
            content,
        }
    }

    fn generate_id() -> u64 {
        rand::random()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventContent {
    FeedLagged,
    PipelineCreated(Box<PipelineInfoResponse>),
    JobRestarted {
        pipeline_id: i32,
        old_job_id: i32,
        new_job_id: i32,
    },
    JobCompleted(Box<JobInfoResponse>),
    JobAssigned(Box<JobAssignmentUpdate>),
    JobUnassigned(Box<JobAssignmentUpdate>),
}

impl EventContent {
    pub fn is_related_to_pipeline(&self, id: i32) -> bool {
        match self {
            EventContent::FeedLagged => false,
            EventContent::PipelineCreated(resp) => resp.pipeline_id == id,
            EventContent::JobRestarted { pipeline_id, .. } => *pipeline_id == id,
            EventContent::JobCompleted(resp) => resp.pipeline_id == id,
            EventContent::JobAssigned(update) => update.pipeline_id == id,
            EventContent::JobUnassigned(update) => update.pipeline_id == id,
        }
    }

    pub fn is_related_to_job(&self, id: i32) -> bool {
        match self {
            EventContent::FeedLagged => false,
            EventContent::PipelineCreated(_) => false,
            EventContent::JobRestarted { old_job_id, .. } => *old_job_id == id,
            EventContent::JobCompleted(resp) => resp.job_id == id,
            EventContent::JobAssigned(update) => update.job_id == id,
            EventContent::JobUnassigned(update) => update.job_id == id,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JobAssignmentUpdate {
    pub pipeline_id: i32,
    pub job_id: i32,
    pub arch: String,
    pub worker_id: i32,
    pub worker_name: String,
    pub worker_arch: String,
}
