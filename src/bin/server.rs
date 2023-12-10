use buildit::{ensure_job_queue, Job, JobResult, WorkerHeartbeat, WorkerIdentifier};
use chrono::{DateTime, Local};
use clap::Parser;
use futures::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions},
    types::FieldTable,
    BasicProperties, ConnectionProperties,
};
use log::{error, info, warn};
use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use teloxide::{prelude::*, types::ParseMode, utils::command::BotCommands};

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "BuildIt! supports the following commands:"
)]
enum Command {
    #[command(description = "Display usage: /help")]
    Help,
    #[command(
        description = "Start a build job: /build [git-ref] [packages] [archs] (e.g., /build stable bash,fish amd64,arm64)"
    )]
    Build(String),
    #[command(description = "Start a build job from GitHub PR: /pr [pr-number]")]
    PR(String),
    #[command(description = "Show queue and server status: /status")]
    Status,
}

struct WorkerStatus {
    last_heartbeat: DateTime<Local>,
}

static WORKERS: Lazy<Arc<Mutex<HashMap<WorkerIdentifier, WorkerStatus>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

async fn build_inner(
    git_ref: &str,
    packages: &Vec<String>,
    archs: &Vec<&str>,
    github_pr: Option<u64>,
    msg: &Message,
) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(&ARGS.amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    // for each arch, create a job
    for arch in archs {
        let job = Job {
            packages: packages.iter().map(|s| s.to_string()).collect(),
            git_ref: git_ref.to_string(),
            arch: arch.to_string(),
            tg_chatid: msg.chat.id,
            github_pr,
        };

        info!("Adding job to message queue {:?} ...", job);

        // each arch has its own queue
        let queue_name = format!("job-{}", job.arch);
        ensure_job_queue(&queue_name, &channel).await?;

        channel
            .basic_publish(
                "",
                &queue_name,
                BasicPublishOptions::default(),
                &serde_json::to_vec(&job)?,
                BasicProperties::default(),
            )
            .await?
            .await?;
    }
    Ok(())
}

async fn build(
    bot: &Bot,
    git_ref: &str,
    packages: &Vec<String>,
    archs: &Vec<&str>,
    github_pr: Option<u64>,
    msg: &Message,
) -> ResponseResult<()> {
    let mut archs = archs.clone();
    if archs.contains(&"mainline") {
        // follow https://github.com/AOSC-Dev/autobuild3/blob/master/sets/arch_groups/mainline
        archs.extend_from_slice(&[
            "amd64",
            "arm64",
            "loongarch64",
            "loongson3",
            "mips64r6el",
            "ppc64el",
            "riscv64",
        ]);
        archs.retain(|arch| *arch != "mainline");
    }
    archs.sort();
    archs.dedup();

    match build_inner(git_ref, &packages, &archs, github_pr, &msg).await {
        Ok(()) => {
            bot.send_message(
                            msg.chat.id,
                            format!(
                                "\n__*New Job Summary*__\n\n*Git reference*: {}\n{}*Architecture\\(s\\)*: {}\n*Package\\(s\\)*: {}\n",
                                teloxide::utils::markdown::escape(git_ref),
                                if let Some(pr) = github_pr { format!("*GitHub PR*: [\\#{}](https://github.com/AOSC-Dev/aosc-os-abbs/pull/{})\n", pr, pr) } else { String::new() },
                                archs.join(", "),
                                teloxide::utils::markdown::escape(&packages.join(", ")),
                            ),
                        ).parse_mode(ParseMode::MarkdownV2)
                        .await?;
        }
        Err(err) => {
            bot.send_message(msg.chat.id, format!("Failed to create job: {}", err))
                .await?;
        }
    }
    Ok(())
}

async fn status(args: &Args) -> anyhow::Result<String> {
    let mut res = String::from("__*Queue Status*__\n\n");
    let conn = lapin::Connection::connect(&ARGS.amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    for arch in [
        "amd64",
        "arm64",
        "loongarch64",
        "loongson3",
        "mips64r6el",
        "ppc64el",
        "riscv64",
    ] {
        let queue_name = format!("job-{}", arch);

        let queue = ensure_job_queue(&queue_name, &channel).await?;

        // read unacknowledged job count
        let mut unacknowledged_str = String::new();
        if let Some(api) = &args.rabbitmq_queue_api {
            let client = reqwest::Client::new();
            let res = client
                .get(format!("{}{}", api, queue_name))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;
            if let Some(unacknowledged) = res
                .as_object()
                .and_then(|m| m.get("messages_unacknowledged"))
                .and_then(|v| v.as_i64())
            {
                unacknowledged_str = format!("{} job\\(s\\), ", unacknowledged);
            }
        }
        res += &format!(
            "*{}*: {}{} available server\\(s\\)\n",
            teloxide::utils::markdown::escape(&arch),
            unacknowledged_str,
            queue.consumer_count()
        );
    }

    res += "\n__*Server Status*__\n\n";
    let fmt = timeago::Formatter::new();
    if let Ok(lock) = WORKERS.lock() {
        for (identifier, status) in lock.iter() {
            res += &teloxide::utils::markdown::escape(&format!(
                "{} ({}): Online as of {}\n",
                identifier.hostname,
                identifier.arch,
                fmt.convert_chrono(status.last_heartbeat, Local::now())
            ));
        }
    }
    Ok(res)
}

async fn answer(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::PR(arguments) => {
            if let Ok(pr_number) = str::parse::<u64>(&arguments) {
                match octocrab::instance()
                    .pulls("AOSC-Dev", "aosc-os-abbs")
                    .get(pr_number)
                    .await
                {
                    Ok(pr) => {
                        let git_ref = &pr.head.ref_field;
                        // find lines starting with #buildit
                        let packages: Vec<String> = pr
                            .body
                            .and_then(|body| {
                                body.lines()
                                    .filter(|line| line.starts_with("#buildit"))
                                    .map(|line| {
                                        line.split(" ")
                                            .map(str::to_string)
                                            .skip(1)
                                            .collect::<Vec<_>>()
                                    })
                                    .next()
                            })
                            .unwrap_or_else(Vec::new);
                        if packages.len() > 0 {
                            let archs = vec![
                                "amd64",
                                "arm64",
                                "loongson3",
                                "mips64r6el",
                                "ppc64el",
                                "riscv64",
                            ];
                            build(&bot, git_ref, &packages, &archs, Some(pr_number), &msg).await?;
                        } else {
                            bot.send_message(msg.chat.id, format!("Please list packages to build in pr info starting with '#buildit'."))
                                .await?;
                        }
                    }
                    Err(err) => {
                        bot.send_message(msg.chat.id, format!("Failed to get pr info: {err}."))
                            .await?;
                    }
                }
            } else {
                bot.send_message(
                    msg.chat.id,
                    format!("Got invalid pr description: {arguments}."),
                )
                .await?;
            }
        }
        Command::Build(arguments) => {
            let parts: Vec<&str> = arguments.split(" ").collect();
            if parts.len() == 3 {
                let git_ref = parts[0];
                let packages: Vec<String> = parts[1].split(",").map(str::to_string).collect();
                let archs: Vec<&str> = parts[2].split(",").collect();
                build(&bot, git_ref, &packages, &archs, None, &msg).await?;
                return Ok(());
            }

            bot.send_message(
                msg.chat.id,
                format!("Got invalid job description: {arguments}."),
            )
            .await?;
        }
        Command::Status => match status(&ARGS).await {
            Ok(status) => {
                bot.send_message(msg.chat.id, status)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
            }
            Err(err) => {
                bot.send_message(msg.chat.id, format!("Failed to get status: {}", err))
                    .await?;
            }
        },
    };

    Ok(())
}

/// Observe job completion messages
pub async fn job_completion_worker_inner(bot: Bot, amqp_addr: &str) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    let _queue = channel
        .queue_declare(
            "job-completion",
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await?;

    let mut consumer = channel
        .basic_consume(
            "job-completion",
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
            info!("Processing job result {:?} ...", result);
            let success = result.successful_packages == result.job.packages;
            // Report job result to user
            bot.send_message(
                result.job.tg_chatid,
                format!(
                    "{} Job completed on {} \\({}\\)\n\n*Time elapsed*: {}\n{}{}*Architecture*: {}\n*Package\\(s\\) to build*: {}\n*Package\\(s\\) successfully built*: {}\n*Package\\(s\\) failed to build*: {}\n\n[Build Log \\>\\>]({})\n",
                    if success { "✅️" } else { "❌" },
                    teloxide::utils::markdown::escape(&result.worker.hostname),
                    result.worker.arch,
                    teloxide::utils::markdown::escape(&format!("{:.2?}", result.elapsed)),
                    if let Some(git_commit) = &result.git_commit {
                        format!("*Git commit*: [{}](https://github.com/AOSC-Dev/aosc-os-abbs/commit/{})\n", &git_commit[..8], git_commit)
                    } else {
                        String::new()
                    },
                    if let Some(pr) = result.job.github_pr {
                        format!("*GitHub PR*: [\\#{}](https://github.com/AOSC-Dev/aosc-os-abbs/pull/{})\n", pr, pr)
                    } else {
                        String::new()
                    },
                    result.job.arch,
                    teloxide::utils::markdown::escape(&result.job.packages.join(", ")),
                    teloxide::utils::markdown::escape(&result.successful_packages.join(", ")),
                    teloxide::utils::markdown::escape(&result.failed_package.clone().unwrap_or(String::from("None"))),
                    result.log.clone().unwrap_or(String::from("None")),
                ),
            ).parse_mode(ParseMode::MarkdownV2)
            .await?;

            // if associated with github pr, update comments
            if let Some(github_access_token) = &ARGS.github_access_token {
                if let Some(pr) = result.job.github_pr {
                    let new_content = format!(
                        "{} Job completed on {} \\({}\\)\n\n**Time elapsed**: {}\n{}**Architecture**: {}\n**Package\\(s\\) to build**: {}\n**Package\\(s\\) successfully built**: {}\n**Package\\(s\\) failed to build**: {}\n\n[Build Log \\>\\>]({})\n",
                        if success { "✅️" } else { "❌" },
                        result.worker.hostname,
                        result.worker.arch,
                        format!("{:.2?}", result.elapsed),
                        if let Some(git_commit) = &result.git_commit {
                            format!("**Git commit**: [{}](https://github.com/AOSC-Dev/aosc-os-abbs/commit/{})\n", &git_commit[..8], git_commit)
                        } else {
                            String::new()
                        },
                        result.job.arch,
                        teloxide::utils::markdown::escape(&result.job.packages.join(", ")),
                        teloxide::utils::markdown::escape(&result.successful_packages.join(", ")),
                        teloxide::utils::markdown::escape(&result.failed_package.clone().unwrap_or(String::from("None"))),
                        result.log.unwrap_or(String::from("None")),
                    );

                    // update or create new comment
                    let page = octocrab::instance()
                        .issues("AOSC-Dev", "aosc-os-abbs")
                        .list_comments(pr)
                        .send()
                        .await?;

                    let crab = octocrab::Octocrab::builder()
                        .user_access_token(github_access_token.clone())
                        .build()?;

                    // TODO: handle paging
                    let mut found = false;
                    for comment in page {
                        // find existing comment generated by @aosc-buildit-bot
                        info!("Got comment {:?}", comment);
                        if comment.user.login == "aosc-buildit-bot" {
                            // found, append new data
                            found = true;

                            let mut body = String::new();
                            if let Some(orig) = &comment.body {
                                body += orig;
                                body += "\n";
                            }
                            body += &new_content;

                            crab.issues("AOSC-Dev", "aosc-os-abbs")
                                .update_comment(comment.id, body)
                                .await?;
                            break;
                        }
                    }

                    if !found {
                        crab.issues("AOSC-Dev", "aosc-os-abbs")
                            .create_comment(pr, new_content)
                            .await?;
                    }
                }
            }
        }

        // finish
        if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
            warn!(
                "Failed to delete job result {:?}, error: {:?}",
                delivery, err
            );
        } else {
            info!("Finished processing job result {:?}", delivery.delivery_tag);
        }
    }
    Ok(())
}

pub async fn job_completion_worker(bot: Bot, amqp_addr: String) -> anyhow::Result<()> {
    loop {
        info!("Starting job completion worker ...");
        if let Err(err) = job_completion_worker_inner(bot.clone(), &amqp_addr).await {
            error!("Got error while starting job completion worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

pub async fn heartbeat_worker_inner(amqp_addr: String) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(&amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    let queue_name = "worker-heartbeat";
    ensure_job_queue(&queue_name, &channel).await?;

    let mut consumer = channel
        .basic_consume(
            &queue_name,
            "worker-heartbeat",
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

        if let Some(heartbeat) = serde_json::from_slice::<WorkerHeartbeat>(&delivery.data).ok() {
            info!("Processing worker heartbeat {:?} ...", heartbeat);

            // update worker status
            if let Ok(mut lock) = WORKERS.lock() {
                if let Some(status) = lock.get_mut(&heartbeat.identifier) {
                    status.last_heartbeat = Local::now();
                } else {
                    lock.insert(
                        heartbeat.identifier.clone(),
                        WorkerStatus {
                            last_heartbeat: Local::now(),
                        },
                    );
                }
            }

            // finish
            if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
                warn!("Failed to ack heartbeat {:?}, error: {:?}", delivery, err);
            } else {
                info!("Finished ack-ing heartbeat {:?}", delivery.delivery_tag);
            }
        }
    }

    Ok(())
}

pub async fn heartbeat_worker(amqp_addr: String) -> anyhow::Result<()> {
    loop {
        info!("Starting heartbeat worker ...");
        if let Err(err) = heartbeat_worker_inner(amqp_addr.clone()).await {
            error!("Got error while starting heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// AMQP address to access message queue
    #[arg(env = "BUILDIT_AMQP_ADDR")]
    amqp_addr: String,

    /// RabbitMQ address to access queue api e.g. http://user:password@host:port/api/queues/vhost/
    #[arg(env = "BUILDIT_RABBITMQ_QUEUE_API")]
    rabbitmq_queue_api: Option<String>,

    /// GitHub access token
    #[arg(env = "BUILDIT_GITHUB_ACCESS_TOKEN")]
    github_access_token: Option<String>,
}

static ARGS: Lazy<Args> = Lazy::new(|| Args::parse());

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Starting AOSC BuildIt! server with args {:?}", *ARGS);

    let bot = Bot::from_env();

    tokio::spawn(heartbeat_worker(ARGS.amqp_addr.clone()));

    tokio::spawn(job_completion_worker(bot.clone(), ARGS.amqp_addr.clone()));

    Command::repl(bot, answer).await;
}
