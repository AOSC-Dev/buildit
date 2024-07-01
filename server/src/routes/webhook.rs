use anyhow::{anyhow, bail};
use axum::{extract::State, Json};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;
use tracing::{info, warn};

use crate::{api, formatter::to_html_new_pipeline_summary, DbPool, ARGS};

use super::{AnyhowError, AppState};

#[derive(Debug, Deserialize)]
pub struct GithubWebhook {
    comment: Option<Comment>,
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

pub async fn webhook_handler(
    State(state): State<AppState>,
    Json(json): Json<Value>,
) -> Result<(), AnyhowError> {
    info!("Got Github webhook request: {}", json);
    let webhook: Result<GithubWebhook, serde_json::Error> = serde_json::from_value(json);

    if let Ok(webhook) = webhook {
        if let Some(comment) = webhook.comment {
            tokio::spawn(async move {
                let res = handle_webhook_comment(&comment, state.pool).await;
                if let Err(err) = res {
                    warn!("Failed to handle webhook comment: {}", err);
                }
            });
        }
    }

    Ok(())
}

async fn handle_webhook_comment(comment: &Comment, pool: DbPool) -> anyhow::Result<()> {
    let is_org_user = is_org_user(&comment.user.login).await?;

    if !is_org_user {
        return Ok(());
    }

    let body = comment.body.split_whitespace();

    let num = comment
        .issue_url
        .split('/')
        .last()
        .and_then(|x| x.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Failed to get pr number"))?;

    let mut is_request = false;
    for i in body {
        if is_request {
            match i {
                "build" => {
                    let res =
                        api::pipeline_new_pr(pool, num, None, api::JobSource::Github(num)).await?;

                    let crab = octocrab::Octocrab::builder()
                        .user_access_token(ARGS.github_access_token.clone())
                        .build()?;

                    let summary = to_html_new_pipeline_summary(
                        res.id,
                        &res.git_branch,
                        &res.git_sha,
                        res.github_pr.map(|n| n as u64),
                        &res.archs.split(',').collect::<Vec<_>>(),
                        &res.packages.split(',').collect::<Vec<_>>(),
                    );

                    crab.issues("aosc-dev", "aosc-os-abbs")
                        .create_comment(num, summary)
                        .await?;
                }
                x => {
                    warn!("Unsupport request: {x}")
                }
            }
            break;
        }
        if i == "@aosc-buildit-bot" {
            is_request = true;
        }
    }

    Ok(())
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
