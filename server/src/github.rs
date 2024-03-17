use crate::ARGS;
use octocrab::models::pulls::PullRequest;
use octocrab::{models::InstallationId, Octocrab};
use serde::{Deserialize, Serialize};
use teloxide::types::{ChatId, Message};

#[derive(Deserialize, Serialize, Debug)]
pub struct GithubToken {
    pub access_token: String,
    pub expires_in: i64,
    pub refresh_token: String,
    pub refresh_token_expires_in: i64,
    pub scope: String,
    pub token_type: String,
}

pub async fn login_github(
    msg: &Message,
    arguments: String,
) -> Result<reqwest::Response, reqwest::Error> {
    let client = reqwest::Client::new();

    client
        .get("https://minzhengbu.aosc.io/login_from_telegram".to_string())
        .query(&[
            ("telegram_id", msg.chat.id.0.to_string()),
            ("rid", arguments),
        ])
        .send()
        .await
        .and_then(|x| x.error_for_status())
}

#[tracing::instrument(skip(secret))]
pub async fn get_github_token(msg_chatid: &ChatId, secret: &str) -> anyhow::Result<GithubToken> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://minzhengbu.aosc.io/get_token")
        .query(&[("id", &msg_chatid.0.to_string())])
        .header("secret", secret)
        .send()
        .await
        .and_then(|x| x.error_for_status())?;

    let token = resp.json().await?;

    Ok(token)
}

/// Collect packages to build from pull request
pub fn get_packages_from_pr(pr: &PullRequest) -> Vec<String> {
    pr.body
        .as_ref()
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
        .unwrap_or_default()
}

/// Create octocrab instance authenticated as github installation
pub async fn get_crab_github_installation() -> anyhow::Result<Option<Octocrab>> {
    if let Some(id) = ARGS
        .github_app_id
        .as_ref()
        .and_then(|x| x.parse::<u64>().ok())
    {
        if let Some(app_private_key) = ARGS.github_app_key.as_ref() {
            let key = tokio::fs::read(app_private_key).await?;
            let key =
                tokio::task::spawn_blocking(move || jsonwebtoken::EncodingKey::from_rsa_pem(&key))
                    .await??;

            let app_crab = octocrab::Octocrab::builder().app(id.into(), key).build()?;
            // TODO: move to config
            return Ok(Some(
                app_crab
                    .installation_and_token(InstallationId(45135446))
                    .await?
                    .0,
            ));
        }
    }
    Ok(None)
}
