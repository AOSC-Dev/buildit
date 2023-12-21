use crate::ARGS;
use anyhow::{anyhow, bail};
use common::{JobError, JobResult};
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
            info!("Processing job result {:?} ...", result);
            let success = result.successful_packages == result.job.packages;
            // Report job result to user
            bot.send_message(
                result.job.tg_chatid,
                format!(
                    "{} Job completed on {} \\({}\\)\n\n*Time elapsed*: {}\n{}{}*Architecture*: {}\n*Package\\(s\\) to build*: {}\n*Package\\(s\\) successfully built*: {}\n*Package\\(s\\) failed to build*: {}\n*Package\\(s\\) not built due to previous build failure*: {}\n\n[Build Log \\>\\>]({})\n",
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
                    teloxide::utils::markdown::escape(&result.skipped_packages.join(", ")),
                    result.log.clone().unwrap_or(String::from("None")),
                ),
            ).parse_mode(ParseMode::MarkdownV2)
            .await?;

            // if associated with github pr, update comments
            if let Some(github_access_token) = &ARGS.github_access_token {
                if let Some(pr) = result.job.github_pr {
                    let new_content = format!(
                        "{} Job completed on {} \\({}\\)\n\n**Time elapsed**: {}\n{}**Architecture**: {}\n**Package\\(s\\) to build**: {}\n**Package\\(s\\) successfully built**: {}\n**Package\\(s\\) failed to build**: {}\n**Package\\(s\\) not built due to previous build failure**: {}\n\n[Build Log \\>\\>]({})\n",
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
                        teloxide::utils::markdown::escape(&result.skipped_packages.join(", ")),
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

                        let pr_arch = match result.job.arch.as_str() {
                            "amd64" => "AMD64 `amd64`",
                            "arm64" => "AArch64 `arm64`",
                            "noarch" => "Architecture-independent `noarch`",
                            "loongson3" => "Loongson 3 `loongson3`",
                            "mips64r6el" => "MIPS R6 64-bit (Little Endian) `mips64r6el`",
                            "ppc64el" => "PowerPC 64-bit (Little Endian) `ppc64el`",
                            "riscv64" => "RISC-V 64-bit `riscv64`",
                            _ => bail!("Unknown architecture"),
                        };

                        let body =
                            body.replace(&format!("- [ ] {pr_arch}"), &format!("- [x] {pr_arch}"));

                        crab.pulls("AOSC-Dev", "aosc-os-abbs")
                            .update(pr)
                            .body(body)
                            .send()
                            .await?;
                    }
                }
            }
        }

        if let Some(result) = serde_json::from_slice::<JobError>(&delivery.data).ok() {
            bot.send_message(result.job.tg_chatid, result.error).await?;
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
