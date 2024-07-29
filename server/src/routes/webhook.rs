use anyhow::{anyhow, bail};
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
                    let res = handle_webhook_comment(&webhook_comment.comment, pool).await;
                    if let Err(err) = res {
                        warn!("Failed to handle webhook comment: {}", err);
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

async fn handle_webhook_comment(comment: &Comment, pool: DbPool) -> anyhow::Result<()> {
    let is_org_user = is_org_user(&comment.user.login).await?;

    if !is_org_user {
        return Ok(());
    }

    let body = comment.body.split_whitespace().collect::<Vec<_>>();

    let num = comment
        .issue_url
        .split('/')
        .last()
        .and_then(|x| x.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Failed to get pr number"))?;

    let mut is_request = false;

    for (i, c) in body.iter().enumerate() {
        if is_request {
            match c.to_owned() {
                "build" => {
                    let mut archs = None;
                    if let Some(v) = body.get(i + 1) {
                        archs = Some(v.to_owned());
                    }

                    pipeline_new_pr_impl(pool, num, archs).await?;
                }
                x => {
                    warn!("Unsupport request: {x}")
                }
            }
            break;
        }
        if *c == "@aosc-buildit-bot" {
            is_request = true;
        }
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
