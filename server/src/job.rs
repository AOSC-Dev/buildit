use crate::{
    formatter::{to_html_build_result, to_markdown_build_result},
    github::{AMD64, ARM64, LOONGSON3, MIPS64R6EL, NOARCH, PPC64EL, RISCV64},
    ARGS,
};
use common::{JobError, JobOk, JobResult};
use futures::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicConsumeOptions, QueueDeclareOptions},
    types::FieldTable,
    ConnectionProperties,
};
use log::{error, info, warn};
use std::time::Duration;
use teloxide::{prelude::*, types::ParseMode};

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

    let mut retry = None;

    while let Some(delivery) = consumer.next().await {
        let delivery = match delivery {
            Ok(delivery) => delivery,
            Err(err) => {
                error!("Got error in lapin delivery: {}", err);
                continue;
            }
        };

        if retry.map(|x| x < 5).unwrap_or(true) {
            match handle_success_message(&delivery, &bot, retry).await {
                HandleSuccessResult::Ok | HandleSuccessResult::DoNotRetry => {
                    ack_delivery(delivery).await
                }
                HandleSuccessResult::Retry(x) => {
                    retry = Some(x);
                    continue;
                }
            }
        } else {
            ack_delivery(delivery).await;
        }
    }
    Ok(())
}

async fn ack_delivery(delivery: lapin::message::Delivery) {
    if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
        warn!(
            "Failed to delete job result {:?}, error: {:?}",
            delivery, err
        );
    } else {
        info!("Finished processing job result {:?}", delivery.delivery_tag);
    }
}

enum HandleSuccessResult {
    Ok,
    Retry(u8),
    DoNotRetry,
}

async fn handle_success_message(
    delivery: &lapin::message::Delivery,
    bot: &Bot,
    retry: Option<u8>,
) -> HandleSuccessResult {
    match serde_json::from_slice::<JobResult>(&delivery.data) {
        Ok(result) => {
            match result {
                JobResult::Ok(job) => {
                    info!("Processing job result {:?} ...", job);

                    let JobOk {
                        job: job_parent,
                        successful_packages,
                        ..
                    } = &job;

                    let success = job_parent
                        .packages
                        .iter()
                        .all(|x| successful_packages.contains(x));

                    let s = to_html_build_result(&job, success);

                    if let Err(e) = bot
                        .send_message(job.job.tg_chatid, &s)
                        .parse_mode(ParseMode::Html)
                        .disable_web_page_preview(true)
                        .await
                    {
                        error!("{}", e);
                        return update_retry(retry);
                    }

                    // if associated with github pr, update comments
                    if let Some(github_access_token) = &ARGS.github_access_token {
                        if let Some(pr_num) = job_parent.github_pr {
                            let new_content = to_markdown_build_result(&job, success);

                            // update or create new comment
                            let page = match octocrab::instance()
                                .issues("AOSC-Dev", "aosc-os-abbs")
                                .list_comments(pr_num)
                                .send()
                                .await
                            {
                                Ok(page) => page,
                                Err(e) => {
                                    error!("{e}");
                                    return update_retry(retry);
                                }
                            };

                            let crab = match octocrab::Octocrab::builder()
                                .user_access_token(github_access_token.clone())
                                .build()
                            {
                                Ok(crab) => crab,
                                Err(e) => {
                                    error!("{e}");
                                    return HandleSuccessResult::DoNotRetry;
                                }
                            };

                            // TODO: handle paging
                            let mut found = false;
                            for comment in page {
                                // find existing comment generated by @aosc-buildit-bot
                                if comment.user.login == "aosc-buildit-bot" {
                                    // found, append new data
                                    found = true;
                                    info!("Found existing comment, updating");

                                    let mut body = String::new();
                                    if let Some(orig) = &comment.body {
                                        body += orig;
                                        body += "\n";
                                    }
                                    body += &new_content;

                                    if let Err(e) = crab
                                        .issues("AOSC-Dev", "aosc-os-abbs")
                                        .update_comment(comment.id, body)
                                        .await
                                    {
                                        error!("{e}");
                                        return update_retry(retry);
                                    };
                                    break;
                                }
                            }

                            if !found {
                                info!("No existing comments, create one");
                                if let Err(e) = crab
                                    .issues("AOSC-Dev", "aosc-os-abbs")
                                    .create_comment(pr_num, new_content)
                                    .await
                                {
                                    error!("{e}");
                                    return update_retry(retry);
                                }
                            }

                            if success {
                                let pr = match crab
                                    .pulls("AOSC-Dev", "aosc-os-abbs")
                                    .get(pr_num)
                                    .await
                                {
                                    Ok(pr) => pr,
                                    Err(e) => {
                                        error!("{e}");
                                        return update_retry(retry);
                                    }
                                };

                                let body = if let Some(body) = pr.body {
                                    body
                                } else {
                                    return HandleSuccessResult::DoNotRetry;
                                };

                                let pr_arch = match job_parent.arch.as_str() {
                                    "amd64" if job_parent.noarch => NOARCH,
                                    "amd64" => AMD64,
                                    "arm64" => ARM64,
                                    "loongson3" => LOONGSON3,
                                    "mips64r6el" => MIPS64R6EL,
                                    "ppc64el" => PPC64EL,
                                    "riscv64" => RISCV64,
                                    "loongarch64" => {
                                        // FIXME: loongarch64 does not in mainline for now
                                        return HandleSuccessResult::Ok;
                                    }
                                    x => {
                                        error!("Unknown architecture: {x}");
                                        return HandleSuccessResult::DoNotRetry;
                                    }
                                };

                                let body = body.replace(
                                    &format!("- [ ] {pr_arch}"),
                                    &format!("- [x] {pr_arch}"),
                                );

                                if let Err(e) = crab
                                    .pulls("AOSC-Dev", "aosc-os-abbs")
                                    .update(pr_num)
                                    .body(body)
                                    .send()
                                    .await
                                {
                                    error!("{e}");
                                    return update_retry(retry);
                                }
                            }
                        }
                    }
                }
                JobResult::Error(job) => {
                    let JobError { job, worker, error } = job;
                    if let Err(e) = bot
                        .send_message(
                            job.tg_chatid,
                            format!(
                                "{}({}) build packages: {:?} Got Error: {}",
                                worker.hostname, job.arch, job.packages, error
                            ),
                        )
                        .await
                    {
                        error!("{e}");
                        return update_retry(retry);
                    }
                }
            }
        }
        Err(err) => {
            warn!("Got invalid json in job-completion: {}", err);
        }
    }

    HandleSuccessResult::Ok
}

fn update_retry(retry: Option<u8>) -> HandleSuccessResult {
    match retry {
        Some(retry) => HandleSuccessResult::Retry(retry + 1),
        None => HandleSuccessResult::Retry(1),
    }
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
