use axum::{Json, extract::State};
use hyper::HeaderMap;
use serde::Deserialize;
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    api, bot::GitHubUser, formatter::to_html_new_pipeline_summary, is_maintainer, paste_to_aosc_io, DbPool, ARGS
};

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
    user: GitHubUser,
    body: String,
}

#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
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
    pr_num: u64,
    pool: DbPool,
) -> anyhow::Result<()> {
    let mut body = comment.body.split_whitespace();

    // check bot mention first to reduce membership check requests
    let request_bot = body.next().is_some_and(|s| s == "@aosc-buildit-bot");
    if !request_bot {
        return Ok(());
    }

    let is_org_user = is_maintainer(&comment.user.login).await?;

    if !is_org_user {
        return Ok(());
    }

    match body.next() {
        Some("build") => {
            let archs = body.next();

            pipeline_new_pr_impl(pool, pr_num, comment.user.id, archs).await?;
        }
        Some("dickens") => {
            let crab = octocrab::Octocrab::builder()
                .user_access_token(ARGS.github_access_token.clone())
                .build()?;

            let pr = crab.pulls("AOSC-Dev", "aosc-os-abbs").get(pr_num).await?;

            let report =
                dickens::topic::report(&pr.head.ref_field, ARGS.local_repo.clone()).await?;

            if report.len() > 32 * 1024 {
                let id =
                    paste_to_aosc_io(&format!("Dickens-topic report for PR {pr_num}"), &report)
                        .await?;

                crab.issues("AOSC-Dev", "aosc-os-abbs")
                    .create_comment(pr_num, format!("Dickens-topic report has been uploaded to pastebin as [paste {id}](https://aosc.io/paste/detail?id={id})."))
                    .await?;
            } else {
                crab.issues("AOSC-Dev", "aosc-os-abbs")
                    .create_comment(pr_num, report)
                    .await?;
            }
        }
        Some(x) => warn!("Unsupported request: {x}"),
        None => {}
    }

    Ok(())
}

async fn pipeline_new_pr_impl(
    pool: DbPool,
    pr: u64,
    gh_user: i64,
    archs: Option<&str>,
) -> Result<(), anyhow::Error> {
    let res = api::pipeline_new_pr(
        pool,
        pr,
        archs,
        api::JobSource::GitHub { pr, user: gh_user },
        false,
    )
    .await;

    let crab = octocrab::Octocrab::builder()
        .user_access_token(ARGS.github_access_token.clone())
        .build()?;

    let msg = match res {
        Ok((pipeline, jobs)) => to_html_new_pipeline_summary(
            pipeline.id,
            &pipeline.git_branch,
            &pipeline.git_sha,
            pipeline.github_pr.map(|n| n as u64),
            &jobs
                .iter()
                .map(|job| (job.arch.as_str(), job.id))
                .collect::<Vec<_>>(),
            &pipeline.packages.split(',').collect::<Vec<_>>(),
        ),
        Err(e) => {
            format!("Failed to create pipeline: {e}")
        }
    };

    crab.issues("aosc-dev", "aosc-os-abbs")
        .create_comment(pr, msg)
        .await?;

    Ok(())
}
