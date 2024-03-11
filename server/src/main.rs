use axum::extract::MatchedPath;
use axum::http::Method;
use axum::routing::post;
use axum::{routing::get, Router};
use diesel::pg::PgConnection;
use diesel::r2d2::ConnectionManager;
use diesel::r2d2::Pool;
use server::bot::{answer, Command};
use server::recycler::recycler_worker;
use server::routes::{
    dashboard_status, job_list, ping, pipeline_list, pipeline_new_pr, worker_job_update,
    worker_list, worker_poll, AppState,
};
use server::routes::{pipeline_new, worker_heartbeat};
use server::routes::{pipeline_status, worker_status};
use server::{DbPool, ARGS};
use teloxide::prelude::*;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::info_span;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    tracing_subscriber::fmt::init();

    tracing::info!("Connecting to database");
    let manager = ConnectionManager::<PgConnection>::new(&ARGS.database_url);
    let pool = Pool::builder().test_on_check_out(true).build(manager)?;

    let mut handles = vec![];
    let bot = if std::env::var("TELOXIDE_TOKEN").is_ok() {
        tracing::info!("Starting telegram bot");
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

        handles.push(tokio::spawn(async move { telegram.dispatch().await }));
        Some(bot)
    } else {
        None
    };

    tracing::info!("Starting http server");
    // build our application with a route
    let serve_dir = ServeDir::new("frontend/dist")
        .not_found_service(ServeFile::new("frontend/dist/index.html"));
    let state = AppState {
        pool: pool.clone(),
        bot,
    };
    let mut app = Router::new()
        .route("/api/ping", get(ping))
        .route("/api/pipeline/new", post(pipeline_new))
        .route("/api/pipeline/new_pr", post(pipeline_new_pr))
        .route("/api/pipeline/status", get(pipeline_status))
        .route("/api/pipeline/list", get(pipeline_list))
        .route("/api/job/list", get(job_list))
        .route("/api/worker/heartbeat", post(worker_heartbeat))
        .route("/api/worker/poll", post(worker_poll))
        .route("/api/worker/job_update", post(worker_job_update))
        .route("/api/worker/status", get(worker_status))
        .route("/api/worker/list", get(worker_list))
        .route("/api/dashboard/status", get(dashboard_status))
        .fallback_service(serve_dir)
        .with_state(state)
        .layer(
            tower_http::trace::TraceLayer::new_for_http().make_span_with(
                |request: &axum::http::Request<_>| {
                    // learned from https://github.com/tokio-rs/axum/blob/main/examples/tracing-aka-logging/src/main.rs
                    // Log the matched route's path (with placeholders not filled in).
                    // Use request.uri() or OriginalUri if you want the real path.
                    let matched_path = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);

                    info_span!(
                        "http_request",
                        method = ?request.method(),
                        matched_path,
                    )
                },
            ),
        );

    if ARGS.development_mode == Some(true) {
        let cors = CorsLayer::new()
            // allow `GET` and `POST` when accessing the resource
            .allow_methods([Method::GET, Method::POST])
            // allow requests from any origin
            .allow_origin(Any);
        app = app.layer(cors);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    handles.push(tokio::spawn(async {
        axum::serve(listener, app).await.unwrap()
    }));

    handles.push(tokio::spawn(recycler_worker(pool)));

    for handle in handles {
        handle.await?;
    }

    Ok(())
}
