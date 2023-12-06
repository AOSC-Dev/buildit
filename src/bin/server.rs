use buildit::{Job, JobResult};
use clap::Parser;
use futures::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions},
    types::FieldTable,
    BasicProperties, ConnectionProperties,
};
use log::{error, info, warn};
use once_cell::sync::Lazy;
use std::time::Duration;
use teloxide::{prelude::*, utils::command::BotCommands};

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "display usage: /help.")]
    Help,
    #[command(description = "start a job: /build [git-ref] [packages] [archs].")]
    Build(String),
}

async fn answer(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?
        }
        Command::Build(arguments) => {
            let parts: Vec<&str> = arguments.split(" ").collect();
            if parts.len() == 3 {
                let git_ref = parts[0];
                let packages: Vec<&str> = parts[1].split(",").collect();
                let archs: Vec<&str> = parts[2].split(",").collect();

                let conn =
                    lapin::Connection::connect(&ARGS.amqp_addr, ConnectionProperties::default())
                        .await
                        .unwrap();

                let channel = conn.create_channel().await.unwrap();
                let mut reply = String::from("Building for:\n");
                for arch in archs {
                    let job = Job {
                        packages: packages.iter().map(|s| s.to_string()).collect(),
                        git_ref: git_ref.to_string(),
                        arch: arch.to_string(),
                        tg_chatid: msg.chat.id,
                    };
                    reply += &format!("{:?}\n", job);

                    info!("Adding job to message queue {:?}", job);

                    let queue_name = format!("job-{}", job.arch);
                    let _queue = channel
                        .queue_declare(
                            &queue_name,
                            QueueDeclareOptions {
                                durable: true,
                                ..QueueDeclareOptions::default()
                            },
                            FieldTable::default(),
                        )
                        .await
                        .unwrap();

                    channel
                        .basic_publish(
                            "",
                            &queue_name,
                            BasicPublishOptions::default(),
                            &serde_json::to_vec(&job).unwrap(),
                            BasicProperties::default(),
                        )
                        .await
                        .unwrap()
                        .await
                        .unwrap();
                }

                bot.send_message(msg.chat.id, reply).await?;
                return Ok(());
            }

            bot.send_message(
                msg.chat.id,
                format!("Got invalid job description: {arguments}."),
            )
            .await?
        }
    };

    Ok(())
}

pub async fn job_completion_worker_inner(bot: Bot, amqp_addr: &str) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    let _queue = channel
        .queue_declare(
            "job_completion",
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await?;

    let mut consumer = channel
        .basic_consume(
            "job_completion",
            "backend_server",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    while let Some(delivery) = consumer.next().await {
        let delivery = match delivery {
            Ok(delivery) => delivery,
            Err(err) => {
                error!("Got error in lapin delivery: {}", err);
                continue;
            }
        };

        if let Some(result) = serde_json::from_slice::<JobResult>(&delivery.data).ok() {
            info!("Processing job result {:?}", result);
            bot.send_message(
                result.tg_chatid,
                format!("Received job result: {:?}", result),
            )
            .await
            .unwrap();
        }

        // finish
        if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
            warn!(
                "Failed to delete job result {:?} with err {:?}",
                delivery, err
            );
        } else {
            info!("Finish processing job result {:?}", delivery.delivery_tag);
        }
    }
    Ok(())
}

pub async fn job_completion_worker(bot: Bot, amqp_addr: String) -> anyhow::Result<()> {
    loop {
        info!("Starting job completion worker");
        if let Err(err) = job_completion_worker_inner(bot.clone(), &amqp_addr).await {
            error!("Got error running job completion worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// AMQP address to access message queue
    amqp_addr: String,
}

static ARGS: Lazy<Args> = Lazy::new(|| Args::parse());

#[tokio::main]
async fn main() {
    env_logger::init();

    info!("Starting AOSC BuildIt! server with args {:?}", ARGS);

    let bot = Bot::from_env();

    tokio::spawn(job_completion_worker(bot.clone(), ARGS.amqp_addr.clone()));

    Command::repl(bot, answer).await;
}
