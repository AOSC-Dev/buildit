use log::info;
use server::bot::Command;
use server::github_webhooks::get_webhooks_message;
use server::{bot::answer, heartbeat::heartbeat_worker, job::job_completion_worker, ARGS};
use teloxide::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    Ok(())
}
