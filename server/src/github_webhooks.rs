use std::{path::Path, sync::Arc};

use anyhow::anyhow;
use common::JobSource;
use futures::StreamExt;
use lapin::{
    options::{BasicConsumeOptions, QueueDeclareOptions},
    types::FieldTable,
    Channel,
};
use log::{error, info};
use serde::Deserialize;

use crate::{
    bot::build_inner, formatter::to_html_new_job_summary, job::ack_delivery, utils::get_archs, ARGS,
};

#[derive(Debug, Deserialize)]
struct WebhookComment {
    comment: Comment,
}

#[derive(Debug, Deserialize)]
struct Comment {
    issue_url: String,
    user: User,
    body: String,
}

#[derive(Debug, Deserialize)]
struct User {
    login: String,
}

pub async fn get_webhooks_message(channel: Arc<Channel>, path: &Path) -> anyhow::Result<()> {
    let _queue = channel
        .queue_declare(
            "github-webhooks",
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await?;

    let mut consumer = channel
        .basic_consume(
            "github-webhooks",
            "",
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

        if let Ok(comment) = serde_json::from_slice::<WebhookComment>(&delivery.data) {
            info!("Got comment in lapin delivery: {:?}", comment);
            if !comment.comment.body.starts_with("@aosc-buildit-bot") {
                ack_delivery(delivery).await;
                continue;
            }

            let body = comment
                .comment
                .body
                .split_ascii_whitespace()
                .skip(1)
                .collect::<Vec<_>>();

            info!("{body:?}");

            if body[0] != "build" {
                ack_delivery(delivery).await;
                continue;
            }

            let num = match comment
                .comment
                .issue_url
                .split('/')
                .last()
                .and_then(|x| x.parse::<u64>().ok())
                .ok_or_else(|| anyhow!("Failed to get pr number"))
            {
                Ok(num) => num,
                Err(e) => {
                    ack_delivery(delivery).await;
                    return Err(e);
                }
            };

            let pr = match octocrab::instance()
                .pulls("AOSC-Dev", "aosc-os-abbs")
                .get(num)
                .await
            {
                Ok(pr) => pr,
                Err(e) => {
                    ack_delivery(delivery).await;
                    return Err(e.into());
                }
            };

            let packages: Vec<String> = pr
                .body
                .and_then(|body| {
                    body.lines()
                        .filter(|line| line.starts_with("#buildit"))
                        .map(|line| {
                            line.trim()
                                .split_ascii_whitespace()
                                .map(str::to_string)
                                .skip(1)
                                .collect::<Vec<_>>()
                        })
                        .next()
                })
                .unwrap_or_else(Vec::new);

            let archs = if let Some(archs) = body.get(1) {
                archs.split(',').collect::<Vec<_>>()
            } else {
                get_archs(path, &packages)
            };

            let git_ref = if pr.merged_at.is_some() {
                "stable"
            } else {
                &pr.head.ref_field
            };

            let client = reqwest::Client::builder().user_agent("buildit").build()?;

            match client
                .get(format!(
                    "https://api.github.com/orgs/aosc-dev/public_members/{}",
                    comment.comment.user.login
                ))
                .send()
                .await
                .and_then(|x| x.error_for_status())
            {
                Ok(_) => {
                    match build_inner(
                        &git_ref,
                        &packages,
                        &archs,
                        Some(num),
                        JobSource::Github(num),
                        &channel,
                    )
                    .await
                    {
                        Ok(()) => {
                            if let Some(github_access_token) = &ARGS.github_access_token {
                                let crab = match octocrab::Octocrab::builder()
                                    .user_access_token(github_access_token.clone())
                                    .build()
                                {
                                    Ok(v) => v,
                                    Err(e) => {
                                        ack_delivery(delivery).await;
                                        return Err(e.into());
                                    }
                                };

                                let s =
                                    to_html_new_job_summary(&git_ref, Some(num), &archs, &packages);

                                if let Err(e) = crab
                                    .issues("AOSC-Dev", "aosc-os-abbs")
                                    .create_comment(num, s)
                                    .await
                                {
                                    ack_delivery(delivery).await;
                                    return Err(e.into());
                                }
                            }
                        }
                        Err(e) => {
                            ack_delivery(delivery).await;
                            error!("{e}");
                        }
                    }
                }
                Err(e) => {
                    error!("{e}");
                    error!("{} is not a org user", comment.comment.user.login);
                    ack_delivery(delivery).await;
                    continue;
                }
            }
        }
    }

    Ok(())
}
