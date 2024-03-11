use crate::ARGS;

use octocrab::{models::InstallationId, Octocrab};

use teloxide::prelude::*;

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
    return Ok(None);
}
