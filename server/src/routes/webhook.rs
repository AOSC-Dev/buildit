use anyhow::bail;
use axum::{extract::State, Json};
use hyper::HeaderMap;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;
use tracing::{info, warn};

use crate::{api, formatter::to_html_new_pipeline_summary, DbPool, ARGS};

use super::{AnyhowError, AppState};

#[derive(Debug, Deserialize)]
pub struct WebhookComment {
    action: String,
    comment: Comment,
    issue: Issue,
}

#[derive(Debug, Deserialize)]
struct Comment {
    // issue_url: String,
    user: User,
    body: String,
}

#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
}

#[derive(Debug, Deserialize)]
struct User {
    login: String,
}

pub async fn webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(json): Json<Value>,
) -> Result<(), AnyhowError> {
    info!("Got Github webhook request: {}", json);

    match headers.get("X-GitHub-Event").and_then(|x| x.to_str().ok()) {
        Some("issue_comment") => {
            let webhook_comment: WebhookComment = serde_json::from_value(json)?;
            let pool = state.pool;

            if webhook_comment.action == "created" {
                tokio::spawn(async move {
                    if let Err(e) = handle_webhook_comment(
                        &webhook_comment.comment,
                        webhook_comment.issue.number,
                        pool,
                    )
                    .await
                    {
                        warn!("Failed to handle webhook comment: {}", e);
                    }
                });
            }
        }
        x => {
            warn!("Unsupported Github event: {:?}", x);
        }
    }

    Ok(())
}

async fn handle_webhook_comment(
    comment: &Comment,
    issue_num: u64,
    pool: DbPool,
) -> anyhow::Result<()> {
    let is_org_user = is_org_user(&comment.user.login).await?;

    if !is_org_user {
        return Ok(());
    }

    let mut body = comment.body.split_whitespace();

    let request_bot = body.next().is_some_and(|s| s == "@aosc-buildit-bot");

    if !request_bot {
        return Ok(());
    }

    match body.next() {
        Some("build") => {
            let archs = if let Some(archs) = body.next() {
                Some(archs)
            } else {
                None
            };

            pipeline_new_pr_impl(pool, issue_num, archs).await?;
        }
        Some(x) => warn!("Unsupported request: {x}"),
        None => {}
    }

    Ok(())
}

async fn pipeline_new_pr_impl(
    pool: DbPool,
    num: u64,
    archs: Option<&str>,
) -> Result<(), anyhow::Error> {
    let res = api::pipeline_new_pr(pool, num, archs, api::JobSource::Github(num)).await;

    let crab = octocrab::Octocrab::builder()
        .user_access_token(ARGS.github_access_token.clone())
        .build()?;

    let msg = match res {
        Ok(res) => to_html_new_pipeline_summary(
            res.id,
            &res.git_branch,
            &res.git_sha,
            res.github_pr.map(|n| n as u64),
            &res.archs.split(',').collect::<Vec<_>>(),
            &res.packages.split(',').collect::<Vec<_>>(),
        ),
        Err(e) => {
            format!("Failed to create pipeline: {e}")
        }
    };

    crab.issues("aosc-dev", "aosc-os-abbs")
        .create_comment(num, msg)
        .await?;

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
