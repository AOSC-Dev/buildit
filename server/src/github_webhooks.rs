use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{anyhow, bail};
use common::JobSource;
use futures::StreamExt;
use lapin::{
    options::{BasicConsumeOptions, QueueDeclareOptions},
    types::FieldTable,
    Channel,
};
use log::{error, info};
use octocrab::Octocrab;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::{
    formatter::to_html_new_job_summary,
    github::{get_packages_from_pr, update_abbs},
    job::{ack_delivery, send_build_request, update_retry, HandleSuccessResult},
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

pub async fn get_webhooks_message(channel: Arc<Channel>, path: &Path) {
    loop {
        if let Err(e) = get_webhooks_message_inner(channel.clone(), path).await {
            error!("Error getting webhooks message: {e}");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn get_webhooks_message_inner(channel: Arc<Channel>, path: &Path) -> anyhow::Result<()> {
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
        .trim()
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

    let packages = get_packages_from_pr(&pr);

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

    let is_org_user = is_org_user(&comment.comment.user.login).await;

    match is_org_user {
        Ok(true) => (),
        Ok(false) => {
            error!("{} is not a org user", comment.comment.user.login);
            return HandleSuccessResult::DoNotRetry;
        }
        Err(e) => {
            error!("{e}");
            return update_retry(retry);
        }
    }

    let crab = match octocrab::Octocrab::builder()
        .user_access_token(ARGS.github_access_token.clone())
        .build()
    {
        Ok(v) => v,
        Err(e) => {
            error!("{e}");
            return HandleSuccessResult::DoNotRetry;
        }
    };

    if let Err(e) = update_abbs(git_ref).await {
        create_github_comment(&crab, retry, num, &e.to_string()).await;
    }

    let s = to_html_new_job_summary(git_ref, Some(num), &archs, &packages);

    match send_build_request(
        git_ref,
        &packages,
        &archs,
        Some(num),
        JobSource::Github(num),
        &pr.head.sha,
        channel,
    )
    .await
    {
        Ok(()) => {
            let comments = crab
                .issues("AOSC-Dev", "aosc-os-abbs")
                .list_comments(num)
                .send()
                .await;

            let comments = match comments {
                Ok(c) => c,
                Err(e) => {
                    error!("{e}");
                    return update_retry(retry);
                }
            };

            for c in comments {
                if c.user.login == "aosc-buildit-bot" {
                    if let Err(e) = crab
                        .issues("AOSC-Dev", "aosc-os-abbs")
                        .delete_comment(c.id)
                        .await
                    {
                        error!("{e}");
                        return update_retry(retry);
                    }
                }
            }

            create_github_comment(&crab, retry, num, &s).await
        }
        Err(e) => {
            error!("{e}");
            update_retry(retry)
        }
    }
}

async fn create_github_comment(
    crab: &Octocrab,
    retry: Option<u8>,
    num: u64,
    s: &str,
) -> HandleSuccessResult {
    if let Err(e) = crab
        .issues("AOSC-Dev", "aosc-os-abbs")
        .create_comment(num, s)
        .await
    {
        error!("{e}");
        return update_retry(retry);
    }

    HandleSuccessResult::Ok
}

async fn is_org_user(user: &str) -> anyhow::Result<bool> {
    let client = reqwest::Client::builder().user_agent("buildit").build()?;

    let resp = client
        .get(format!(
            "https://api.github.com/orgs/aosc-dev/public_members/{}",
            user
        ))
        .send()
        .await
        .and_then(|x| x.error_for_status());

    match resp {
        Ok(_) => Ok(true),
        Err(e) => match e.status() {
            Some(StatusCode::NOT_FOUND) => Ok(false),
            _ => bail!("Network is not reachable: {e}"),
        },
    }
}
