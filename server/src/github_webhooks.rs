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
    formatter::to_html_new_job_summary,
    job::{ack_delivery, update_retry, HandleSuccessResult, send_build_request},
    utils::get_archs,
    ARGS,
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

    let mut retry = None;

    while let Some(delivery) = consumer.next().await {
        let delivery = match delivery {
            Ok(delivery) => delivery,
            Err(err) => {
                error!("Got error in lapin delivery: {}", err);
                continue;
            }
        };

        if let Ok(comment) = serde_json::from_slice::<WebhookComment>(&delivery.data) {
            match handle_webhook_comment(&comment, path, retry, &channel).await {
                HandleSuccessResult::Ok | HandleSuccessResult::DoNotRetry => {
                    ack_delivery(delivery).await
                }
                HandleSuccessResult::Retry(r) => {
                    if r == 5 {
                        ack_delivery(delivery).await;
                        retry = None;
                        continue;
                    }

                    retry = Some(r);
                }
            }
        }
    }

    Ok(())
}

async fn handle_webhook_comment(
    comment: &WebhookComment,
    path: &Path,
    retry: Option<u8>,
    channel: &Channel,
) -> HandleSuccessResult {
    info!("Got comment in lapin delivery: {:?}", comment);
    if !comment.comment.body.starts_with("@aosc-buildit-bot") {
        return HandleSuccessResult::DoNotRetry;
    }

    let body = comment
        .comment
        .body
        .split_ascii_whitespace()
        .skip(1)
        .collect::<Vec<_>>();

    info!("{body:?}");

    if body[0] != "build" {
        return HandleSuccessResult::DoNotRetry;
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
            error!("{e}");
            return update_retry(retry);
        }
    };

    let pr = match octocrab::instance()
        .pulls("AOSC-Dev", "aosc-os-abbs")
        .get(num)
        .await
    {
        Ok(pr) => pr,
        Err(e) => {
            error!("{e}");
            return update_retry(retry);
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

    let client = match reqwest::Client::builder().user_agent("buildit").build() {
        Ok(c) => c,
        Err(e) => {
            error!("{e}");
            return HandleSuccessResult::DoNotRetry;
        }
    };

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
            match send_build_request(
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
                                error!("{e}");
                                return update_retry(retry);
                            }
                        };

                        let s = to_html_new_job_summary(&git_ref, Some(num), &archs, &packages);

                        if let Err(e) = crab
                            .issues("AOSC-Dev", "aosc-os-abbs")
                            .create_comment(num, s)
                            .await
                        {
                            error!("{e}");
                            return update_retry(retry);
                        }
                    }
                }
                Err(e) => {
                    error!("{e}");
                    return update_retry(retry);
                }
            }
        }
        Err(e) => {
            error!("{e}");
            error!("{} is not a org user", comment.comment.user.login);
            return HandleSuccessResult::DoNotRetry;
        }
    }

    HandleSuccessResult::Ok
}
