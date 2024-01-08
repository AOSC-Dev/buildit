use std::{
    fs,
    path::{Path, PathBuf},
};

use buildit_utils::github::{get_archs, get_repo, open_pr, OpenPRRequest};
use clap::{Parser, Subcommand};
use eyre::{bail, eyre};
use serde::Deserialize;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
pub struct Args {
    #[clap(subcommand)]
    pub subcommand: BiCommand,
    #[arg(short, long)]
    pub abbs_path: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum BiCommand {
    /// Open pull request
    OpenPR {
        #[arg(long)]
        title: String,
        #[arg(short, long)]
        git_ref: Option<String>,
        #[arg(short, long)]
        packages: Vec<String>,
        #[arg(long)]
        tags: Option<Vec<String>>,
    },
    /// Login to Github
    Login,
}

#[derive(Deserialize, Debug)]
pub struct GithubToken {
    pub access_token: String,
    pub expires_in: i64,
    pub refresh_token: String,
    pub refresh_token_expires_in: i64,
    pub scope: String,
    pub token_type: String,
}

#[derive(Deserialize, Debug)]
struct Config {
    pem_path: PathBuf,
    id: String,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    match args.subcommand {
        BiCommand::OpenPR {
            title,
            git_ref,
            packages,
            tags,
        } => {
            let login = dirs_next::data_dir()
                .ok_or_else(|| eyre!("no data dir found!"))?
                .join("github_login");
            let login = fs::read_to_string(login)?;
            let login: GithubToken = serde_json::from_str(&login)?;
            let access_token = &login.access_token;

            let config_dir =
                dirs_next::config_dir().ok_or_else(|| eyre!("no config dir found!"))?;
            let config = config_dir.join("bi_config");

            let s = fs::read_to_string(config)?;

            let config: Config = serde_json::from_str(&s)?;

            match open_pr(
                &config.pem_path,
                access_token,
                config.id.parse::<u64>()?,
                OpenPRRequest {
                    git_ref: if let Some(git_ref) = git_ref {
                        git_ref
                    } else {
                        let repo = get_repo(&args.abbs_path).map_err(|e| eyre!("{e}"))?;
                        repo.head_name()
                            .ok()
                            .and_then(|x| x)
                            .map(|x| x.shorten().to_string())
                            .ok_or_else(|| eyre!("Failed to get branch"))?
                    },
                    abbs_path: args.abbs_path.clone(),
                    packages: packages.join(","),
                    title,
                    tags,
                    archs: get_archs(&args.abbs_path, &packages),
                },
            )
            .await
            {
                Ok(url) => println!("{url}"),
                Err(e) => {
                    eprintln!("{e}");
                }
            }
        }
        BiCommand::Login => {
            println!("Please open url to login Github:");
            println!("https://github.com/login/oauth/authorize?client_id=Iv1.bf26f3e9dd7883ae&redirect_uri=https://minzhengbu.aosc.io/login_cli");
            let input: String = dialoguer::Input::new()
                .with_prompt("JSON")
                .interact_text()?;
            let _: GithubToken = serde_json::from_str(&input)?;

            let dir = dirs_next::data_dir().ok_or_else(|| eyre!("no data dir found!"))?;
            let login_data = dir.join("github_login");
            fs::write(login_data, input)?;

            let pem_path: String = dialoguer::Input::new()
                .with_prompt("PEM Key Path")
                .interact_text()?;

            if !Path::new(&pem_path).exists() {
                bail!("");
            }

            let github_app_id: String = dialoguer::Input::new()
                .with_prompt("Github App ID")
                .interact_text()?;

            let config_dir =
                dirs_next::config_dir().ok_or_else(|| eyre!("no config dir found!"))?;
            let config = config_dir.join("bi_config");

            fs::write(
                config,
                serde_json::json!({
                    "id": github_app_id,
                    "pem_path": pem_path,
                })
                .to_string(),
            )?;
        }
    }

    Ok(())
}
