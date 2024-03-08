use axum::{routing::get, Router};

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    tracing::info!("Starting http server");

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();

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
