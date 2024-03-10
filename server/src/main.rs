use axum::routing::post;
use axum::{routing::get, Router};
use diesel::pg::PgConnection;
use diesel::r2d2::ConnectionManager;
use diesel::r2d2::Pool;
use server::routes::ping;
use server::routes::pipeline_new;
use server::ARGS;
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    tracing_subscriber::fmt::init();

    tracing::info!("Connecting to database");
    let manager = ConnectionManager::<PgConnection>::new(&ARGS.database_url);
    let pool = Pool::builder().test_on_check_out(true).build(manager)?;

    tracing::info!("Starting http server");

    // build our application with a route
    let serve_dir = ServeDir::new("frontend/dist")
        .not_found_service(ServeFile::new("frontend/dist/index.html"));
    let app = Router::new()
        .route("/api/ping", get(ping))
        .route("/api/pipeline/new", post(pipeline_new))
        .fallback_service(serve_dir)
        .with_state(pool)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    tracing::debug!("listening on 127.0.0.1:3000");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;

    /*
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Starting AOSC BuildIt! server with args {:?}", *ARGS);

    let bot = Bot::from_env();

    // setup lapin connection pool
    let mut cfg = deadpool_lapin::Config::default();
    cfg.url = Some(ARGS.amqp_addr.clone());
    let pool = cfg.create_pool(Some(deadpool_lapin::Runtime::Tokio1))?;

    tokio::spawn(heartbeat_worker(pool.clone()));
    tokio::spawn(job_completion_worker(bot.clone(), pool.clone()));

    let handler =
        Update::filter_message().branch(dptree::entry().filter_command::<Command>().endpoint(
            |bot: Bot, pool: deadpool_lapin::Pool, msg: Message, cmd: Command| async move {
                answer(bot, msg, cmd, pool).await
            },
        ));

    let mut telegram = Dispatcher::builder(bot, handler)
        // Pass the shared state to the handler as a dependency.
        .dependencies(dptree::deps![pool.clone()])
        .enable_ctrlc_handler()
        .build();

    tokio::select! {
        v = get_webhooks_message(pool.clone()) => v,
        v = telegram.dispatch() => v,
    };

    */
    Ok(())
}
