use crate::{
    DbPool, HEARTBEAT_TIMEOUT,
    models::{Job, Worker},
};
use anyhow::Context;
use chrono::Utc;
use diesel::{ExpressionMethods, JoinOnDsl, NullableExpressionMethods, QueryDsl, RunQueryDsl};
use std::time::Duration;
use tracing::{info, warn};

pub async fn recycler_worker_inner(pool: DbPool) -> anyhow::Result<()> {
    loop {
        // recycle jobs whose worker is dead
        use crate::schema::{jobs, workers};
        let mut conn = pool
            .get()
            .context("Failed to get db connection from pool")?;

        let deadline = Utc::now() - chrono::Duration::try_seconds(HEARTBEAT_TIMEOUT).unwrap();
        let res = jobs::dsl::jobs
            .inner_join(
                workers::dsl::workers.on(workers::dsl::id
                    .nullable()
                    .eq(jobs::dsl::assigned_worker_id)),
            )
            .filter(workers::dsl::last_heartbeat_time.lt(deadline))
            .load::<(Job, Worker)>(&mut conn)?;

        for (job, worker) in res {
            info!(
                "Job {} was assigned to worker {}, but the worker disappeared",
                job.id, worker.id
            );
            diesel::update(jobs::dsl::jobs.find(job.id))
                .set((
                    jobs::dsl::status.eq("created"),
                    jobs::dsl::assigned_worker_id.eq(None::<i32>),
                ))
                .execute(&mut conn)?;
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

pub async fn recycler_worker(pool: DbPool) {
    loop {
        info!("Starting recycler worker");
        if let Err(err) = recycler_worker_inner(pool.clone()).await {
            warn!("Got error running recycler worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
