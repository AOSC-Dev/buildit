use crate::{DbPool, RemoteAddr, HEARTBEAT_TIMEOUT};
use anyhow::Context;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use diesel::dsl::{count, sum};
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl};
use futures::channel::mpsc::UnboundedSender;
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use teloxide::prelude::*;
use tracing::info;

pub mod job;
pub mod pipeline;
pub mod webhook;
pub mod websocket;
pub mod worker;

pub use job::*;
pub use pipeline::*;
pub use webhook::*;
pub use websocket::*;
pub use worker::*;

pub async fn ping() -> &'static str {
    "PONG"
}

pub struct Viewer {
    remote_addr: RemoteAddr,
    sender: UnboundedSender<axum::extract::ws::Message>,
}

#[derive(Default)]
pub struct WSState {
    last_logs: VecDeque<axum::extract::ws::Message>,
    viewers: Vec<Arc<Viewer>>,
}

// map from hostname to ws state
pub type WSStateMap = Arc<Mutex<HashMap<String, WSState>>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub bot: Option<Bot>,
    pub ws_state_map: WSStateMap,
}

// learned from https://github.com/tokio-rs/axum/blob/main/examples/anyhow-error-response/src/main.rs
pub struct AnyhowError(anyhow::Error);

impl IntoResponse for AnyhowError {
    fn into_response(self) -> Response {
        info!("Returning internal server error for {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", self.0)).into_response()
    }
}

impl<E> From<E> for AnyhowError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

#[derive(Serialize, Default)]
pub struct DashboardStatusResponseByArch {
    total_worker_count: i64,
    live_worker_count: i64,
    total_logical_cores: i64,
    total_memory_bytes: bigdecimal::BigDecimal,

    total_job_count: i64,
    pending_job_count: i64,
    running_job_count: i64,
}

#[derive(Serialize)]
pub struct DashboardStatusResponse {
    total_pipeline_count: i64,

    total_job_count: i64,
    pending_job_count: i64,
    running_job_count: i64,
    finished_job_count: i64,

    total_worker_count: i64,
    live_worker_count: i64,
    total_logical_cores: i64,
    total_memory_bytes: bigdecimal::BigDecimal,

    by_arch: BTreeMap<String, DashboardStatusResponseByArch>,
}

pub async fn dashboard_status(
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<DashboardStatusResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<DashboardStatusResponse, diesel::result::Error, _>(|conn| {
            let total_pipeline_count = crate::schema::pipelines::dsl::pipelines
                .count()
                .get_result(conn)?;
            let total_job_count = crate::schema::jobs::dsl::jobs.count().get_result(conn)?;
            let pending_job_count = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("created"))
                .count()
                .get_result(conn)?;
            let running_job_count = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("running"))
                .count()
                .get_result(conn)?;
            let finished_job_count = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("success"))
                .or_filter(crate::schema::jobs::dsl::status.eq("failed"))
                .count()
                .get_result(conn)?;
            let total_worker_count = crate::schema::workers::dsl::workers
                .filter(crate::schema::workers::dsl::visible.eq(true))
                .count()
                .get_result(conn)?;
            let (total_logical_cores, total_memory_bytes) = crate::schema::workers::dsl::workers
                .select((
                    sum(crate::schema::workers::dsl::logical_cores),
                    sum(crate::schema::workers::dsl::memory_bytes),
                ))
                .filter(crate::schema::workers::dsl::visible.eq(true))
                .get_result::<(Option<i64>, Option<bigdecimal::BigDecimal>)>(conn)?;

            let deadline = Utc::now() - chrono::Duration::try_seconds(HEARTBEAT_TIMEOUT).unwrap();
            let live_worker_count = crate::schema::workers::dsl::workers
                .filter(crate::schema::workers::last_heartbeat_time.gt(deadline))
                .filter(crate::schema::workers::dsl::visible.eq(true))
                .count()
                .get_result(conn)?;

            // collect information by arch
            let mut by_arch: BTreeMap<String, DashboardStatusResponseByArch> = BTreeMap::new();

            for (arch, count, cores, bytes) in crate::schema::workers::dsl::workers
                .group_by(crate::schema::workers::dsl::arch)
                .select((
                    crate::schema::workers::dsl::arch,
                    count(crate::schema::workers::dsl::id),
                    sum(crate::schema::workers::dsl::logical_cores),
                    sum(crate::schema::workers::dsl::memory_bytes),
                ))
                .filter(crate::schema::workers::dsl::visible.eq(true))
                .load::<(String, i64, Option<i64>, Option<bigdecimal::BigDecimal>)>(conn)?
            {
                by_arch.entry(arch.clone()).or_default().total_worker_count = count;
                by_arch.entry(arch.clone()).or_default().total_logical_cores =
                    cores.unwrap_or_default();
                by_arch.entry(arch).or_default().total_memory_bytes = bytes.unwrap_or_default();
            }

            for (arch, count) in crate::schema::workers::dsl::workers
                .filter(crate::schema::workers::last_heartbeat_time.gt(deadline))
                .group_by(crate::schema::workers::dsl::arch)
                .select((
                    crate::schema::workers::dsl::arch,
                    count(crate::schema::workers::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                by_arch.entry(arch).or_default().live_worker_count = count;
            }

            for (arch, count) in crate::schema::jobs::dsl::jobs
                .group_by(crate::schema::jobs::dsl::arch)
                .select((
                    crate::schema::jobs::dsl::arch,
                    count(crate::schema::jobs::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                let arch = if arch == "noarch" || arch == "optenv32" {
                    "amd64".to_string()
                } else {
                    arch
                };
                by_arch.entry(arch).or_default().total_job_count += count;
            }

            for (arch, count) in crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("created"))
                .group_by(crate::schema::jobs::dsl::arch)
                .select((
                    crate::schema::jobs::dsl::arch,
                    count(crate::schema::jobs::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                let arch = if arch == "noarch" || arch == "optenv32" {
                    "amd64".to_string()
                } else {
                    arch
                };
                by_arch.entry(arch).or_default().pending_job_count += count;
            }

            for (arch, count) in crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("running"))
                .group_by(crate::schema::jobs::dsl::arch)
                .select((
                    crate::schema::jobs::dsl::arch,
                    count(crate::schema::jobs::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                let arch = if arch == "noarch" || arch == "optenv32" {
                    "amd64".to_string()
                } else {
                    arch
                };
                by_arch.entry(arch).or_default().running_job_count += count;
            }

            Ok(DashboardStatusResponse {
                total_pipeline_count,
                total_job_count,
                pending_job_count,
                running_job_count,
                finished_job_count,
                total_worker_count,
                live_worker_count,
                total_logical_cores: total_logical_cores.unwrap_or(0),
                total_memory_bytes: total_memory_bytes.unwrap_or_default(),
                by_arch,
            })
        })?,
    ))
}
