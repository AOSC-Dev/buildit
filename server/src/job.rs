use crate::{
    formatter::{to_html_build_result, to_markdown_build_result, FAILED, SUCCESS},
    DbPool, ARGS,
};

use buildit_utils::LOONGARCH64;
use buildit_utils::{AMD64, ARM64, LOONGSON3, MIPS64R6EL, NOARCH, PPC64EL, RISCV64};
use futures::StreamExt;
use octocrab::params::checks::CheckRunConclusion;
use octocrab::params::checks::CheckRunOutput;
use octocrab::{
    models::{CheckRunId, InstallationId},
    Octocrab,
};
use std::time::Duration;
use teloxide::{prelude::*, types::ParseMode};

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