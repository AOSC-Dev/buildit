use crate::{
    github::{AMD64, ARM64, LOONGSON3, MIPS64R6EL, NOARCH, PPC64EL, RISCV64},
    ARGS,
};
use anyhow::{anyhow, bail};
use common::JobResult;
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

    while let Some(delivery) = consumer.next().await {
        let delivery = match delivery {
            Ok(delivery) => delivery,
            Err(err) => {
                error!("Got error in lapin delivery: {}", err);
                continue;
            }
        };

        if let Some(result) = serde_json::from_slice::<JobResult>(&delivery.data).ok() {
            let result_clone = result.clone();
            match result {
                JobResult::Ok {
                    job,
                    successful_packages,
                    failed_package,
                    skipped_packages,
                    log,
                    worker,
                    elapsed,
                    git_commit,
                } => {
                    info!("Processing job result {:?} ...", result_clone);
                    let success = successful_packages == job.packages;
                    // Report job result to user
                    bot.send_message(
                        job.tg_chatid,
                        format!(
                            "{} Job completed on {} \\({}\\)\n\n*Time elapsed*: {}\n{}{}*Architecture*: {}\n*Package\\(s\\) to build*: {}\n*Package\\(s\\) successfully built*: {}\n*Package\\(s\\) failed to build*: {}\n*Package\\(s\\) not built due to previous build failure*: {}\n\n[Build Log \\>\\>]({})\n",
                            if success { "✅️" } else { "❌" },
                            teloxide::utils::markdown::escape(&worker.hostname),
                            worker.arch,
                            teloxide::utils::markdown::escape(&format!("{:.2?}", elapsed)),
                            if let Some(git_commit) = &git_commit {
                                format!("*Git commit*: [{}](https://github.com/AOSC-Dev/aosc-os-abbs/commit/{})\n", &git_commit[..8], git_commit)
                            } else {
                                String::new()
                            },
                            if let Some(pr) = job.github_pr {
                                format!("*GitHub PR*: [\\#{}](https://github.com/AOSC-Dev/aosc-os-abbs/pull/{})\n", pr, pr)
                            } else {
                                String::new()
                            },
                            job.arch,
                            teloxide::utils::markdown::escape(&job.packages.join(", ")),
                            teloxide::utils::markdown::escape(&successful_packages.join(", ")),
                            teloxide::utils::markdown::escape(&failed_package.clone().unwrap_or(String::from("None"))),
                            teloxide::utils::markdown::escape(&skipped_packages.join(", ")),
                            log.clone().unwrap_or(String::from("None")),
                        ),
                    ).parse_mode(ParseMode::MarkdownV2)
                    .await?;

                    // if associated with github pr, update comments
                    if let Some(github_access_token) = &ARGS.github_access_token {
                        if let Some(pr) = job.github_pr {
                            let new_content = format!(
                                "{} Job completed on {} \\({}\\)\n\n**Time elapsed**: {}\n{}**Architecture**: {}\n**Package\\(s\\) to build**: {}\n**Package\\(s\\) successfully built**: {}\n**Package\\(s\\) failed to build**: {}\n**Package\\(s\\) not built due to previous build failure**: {}\n\n[Build Log \\>\\>]({})\n",
                                if success { "✅️" } else { "❌" },
                                worker.hostname,
                                worker.arch,
                                format!("{:.2?}", elapsed),
                                if let Some(git_commit) = &git_commit {
                                    format!("**Git commit**: [{}](https://github.com/AOSC-Dev/aosc-os-abbs/commit/{})\n", &git_commit[..8], git_commit)
                                } else {
                                    String::new()
                                },
                                job.arch,
                                teloxide::utils::markdown::escape(&job.packages.join(", ")),
                                teloxide::utils::markdown::escape(&successful_packages.join(", ")),
                                teloxide::utils::markdown::escape(&failed_package.clone().unwrap_or(String::from("None"))),
                                teloxide::utils::markdown::escape(&skipped_packages.join(", ")),
                                log.unwrap_or(String::from("None")),
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

                                    crab.issues("AOSC-Dev", "aosc-os-abbs")
                                        .update_comment(comment.id, body)
                                        .await?;
                                    break;
                                }
                            }

                            if !found {
                                info!("No existing comments, create one");
                                crab.issues("AOSC-Dev", "aosc-os-abbs")
                                    .create_comment(pr, new_content)
                                    .await?;
                            }

                            if success {
                                let body = crab
                                    .pulls("AOSC-Dev", "aosc-os-abbs")
                                    .get(pr)
                                    .await?
                                    .body
                                    .ok_or_else(|| anyhow!("This PR has no body"))?;

                                let pr_arch = match job.arch.as_str() {
                                    "amd64" => AMD64,
                                    "arm64" => ARM64,
                                    "noarch" => NOARCH,
                                    "loongson3" => LOONGSON3,
                                    "mips64r6el" => MIPS64R6EL,
                                    "ppc64el" => PPC64EL,
                                    "riscv64" => RISCV64,
                                    _ => bail!("Unknown architecture"),
                                };

                                let body = body.replace(
                                    &format!("- [ ] {pr_arch}"),
                                    &format!("- [x] {pr_arch}"),
                                );

                                crab.pulls("AOSC-Dev", "aosc-os-abbs")
                                    .update(pr)
                                    .body(body)
                                    .send()
                                    .await?;
                            }
                        }
                    }
                }
                JobResult::Error { job, worker, error } => {
                    bot.send_message(
                        job.tg_chatid,
                        format!(
                            "{}({}) build packages: {:?} Got Error: {}",
                            worker.hostname, job.arch, job.packages, error
                        ),
                    )
                    .await?;
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
