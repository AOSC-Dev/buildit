use axum::routing::post;
use axum::{routing::get, Router};
use diesel::pg::PgConnection;
use diesel::r2d2::ConnectionManager;
use diesel::r2d2::Pool;
use server::bot::{answer, Command};
use server::routes::{ping, pipeline_new_pr, worker_job_update, worker_poll, AppState};
use server::routes::{pipeline_new, worker_heartbeat};
use server::routes::{pipeline_status, worker_status};
use server::{DbPool, ARGS};
use teloxide::prelude::*;
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    tracing_subscriber::fmt::init();

    tracing::info!("Connecting to database");
    let manager = ConnectionManager::<PgConnection>::new(&ARGS.database_url);
    let pool = Pool::builder().test_on_check_out(true).build(manager)?;

    let bot = Bot::from_env();

    let handler =
        Update::filter_message().branch(dptree::entry().filter_command::<Command>().endpoint(
            |bot: Bot, pool: DbPool, msg: Message, cmd: Command| async move {
                answer(bot, msg, cmd, pool).await
            },
        ));

    let mut telegram = Dispatcher::builder(bot.clone(), handler)
        // Pass the shared state to the handler as a dependency.
        .dependencies(dptree::deps![pool.clone()])
        .enable_ctrlc_handler()
        .build();

    tracing::info!("Starting http server");

    // build our application with a route
    let serve_dir = ServeDir::new("frontend/dist")
        .not_found_service(ServeFile::new("frontend/dist/index.html"));
    let state = AppState { pool, bot };
    let app = Router::new()
        .route("/api/ping", get(ping))
        .route("/api/pipeline/new", post(pipeline_new))
        .route("/api/pipeline/new_pr", post(pipeline_new_pr))
        .route("/api/pipeline/status", get(pipeline_status))
        .route("/api/worker/heartbeat", post(worker_heartbeat))
        .route("/api/worker/poll", post(worker_poll))
        .route("/api/worker/job_update", post(worker_job_update))
        .route("/api/worker/status", get(worker_status))
        .fallback_service(serve_dir)
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    tracing::debug!("listening on 127.0.0.1:3000");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    tokio::spawn(async { axum::serve(listener, app).await.unwrap() });

    telegram.dispatch().await;

    Ok(())
}
