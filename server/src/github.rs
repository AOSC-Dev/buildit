use std::{path::Path, collections::HashMap, borrow::Cow};

use gix::{Repository, sec::{self, trust::DefaultForLevel}, ThreadSafeRepository, prelude::ObjectIdExt};
use jsonwebtoken::EncodingKey;
use log::debug;
use octocrab::models::pulls::PullRequest;
use serde::{Deserialize, Serialize};
use teloxide::types::{ChatId, Message};
use anyhow::{anyhow, bail, Context};
use tokio::{task, process};
use walkdir::WalkDir;

use crate::ARGS;

macro_rules! PR {
    () => {
        "Topic Description\n-----------------\n\n{}Package(s) Affected\n-------------------\n\n{}\n\nSecurity Update?\n----------------\n\nNo\n\nBuild Order\n-----------\n\n```\n{}\n```\n\nTest Build(s) Done\n------------------\n\n**Primary Architectures**\n\n- [ ] AMD64 `amd64`   \n- [ ] AArch64 `arm64`\n \n<!-- - [ ] 32-bit Optional Environment `optenv32` -->\n<!-- - [ ] Architecture-independent `noarch` -->\n\n**Secondary Architectures**\n\n- [ ] Loongson 3 `loongson3`\n- [ ] MIPS R6 64-bit (Little Endian) `mips64r6el`\n- [ ] PowerPC 64-bit (Little Endian) `ppc64el`\n- [ ] RISC-V 64-bit `riscv64`"
    };
}

#[derive(Deserialize, Serialize, Debug)]
pub struct CallbackSecondLoginArgs {
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
    let resp = client
        .get(format!("https://minzhengbu.aosc.io/login_from_telegram"))
        .query(&[
            ("telegram_id", msg.chat.id.0.to_string()),
            ("rid", arguments),
        ])
        .send()
        .await
        .and_then(|x| x.error_for_status());

    resp
}

pub async fn get_github_token(
    msg_chatid: &ChatId,
    secret: &str,
) -> anyhow::Result<CallbackSecondLoginArgs> {
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

pub async fn open_pr(
    parts: Vec<&str>,
    access_token: String,
    secret: &str,
    msg_chatid: ChatId,
    tags: Option<&[String]>,
) -> anyhow::Result<String> {
    let id = ARGS
        .github_app_id
        .as_ref()
        .ok_or_else(|| anyhow!("GITHUB_APP_ID is not set"))?
        .parse::<u64>()?;

    let app_private_key = ARGS
        .github_app_key
        .as_ref()
        .ok_or_else(|| anyhow!("GITHUB_APP_KEY_PEM_PATH is not set"))?;

    let key = tokio::fs::read(app_private_key).await?;
    let key = tokio::task::spawn_blocking(move || jsonwebtoken::EncodingKey::from_rsa_pem(&key))
        .await??;

    update_abbs(parts[1]).await?;
    let path = ARGS
        .abbs_path
        .as_ref()
        .ok_or_else(|| anyhow!("ABBS_PATH_PEM_PATH is not set"))?;

    let commits = task::spawn_blocking(move || get_commits(path)).await??;
    let commits = task::spawn_blocking(move || handle_commits(&commits)).await??;

    let pkgs = parts[2]
        .split(",")
        .map(|x| x.to_string())
        .collect::<Vec<_>>();

    let pkg_affected =
        task::spawn_blocking(move || find_version_by_packages(&pkgs, &path)).await??;
    let pr = open_pr_inner(
        access_token,
        &parts,
        id,
        key.clone(),
        &commits,
        &pkg_affected,
        tags,
    )
    .await;

    match pr {
        Ok(pr) => Ok(pr.html_url.map(|x| x.to_string()).unwrap_or_else(|| pr.url)),
        Err(e) => match e {
            octocrab::Error::GitHub { source, .. }
                if source.message.contains("Bad credentials") =>
            {
                let client = reqwest::Client::new();
                client
                    .get("https://minzhengbu.aosc.io/refresh_token")
                    .header("secret", secret)
                    .query(&[("id", msg_chatid.0.to_string())])
                    .send()
                    .await
                    .and_then(|x| x.error_for_status())?;

                let token = get_github_token(&msg_chatid, secret).await?;
                let pr = open_pr_inner(
                    token.access_token,
                    &parts,
                    id,
                    key,
                    &commits,
                    &pkg_affected,
                    tags,
                )
                .await?;

                Ok(pr.html_url.map(|x| x.to_string()).unwrap_or_else(|| pr.url))
            }
            _ => return Err(e.into()),
        },
    }
}

fn find_version_by_packages(pkgs: &[String], path: &Path) -> anyhow::Result<Vec<String>> {
    let mut res = vec![];
    for i in WalkDir::new(path)
        .max_depth(2)
        .min_depth(2)
        .into_iter()
        .flatten()
    {
        if i.path().is_file() {
            continue;
        }

        let pkg = i.file_name().to_str();

        if pkg.is_none() {
            debug!("Failed to convert str: {}", i.path().display());
            continue;
        }

        let pkg = pkg.unwrap();
        if pkgs.contains(&pkg.to_string()) {
            let spec = i.path().join("spec");
            let defines = i.path().join("autobuild").join("defines");
            let spec = std::fs::read_to_string(spec);
            let defines = std::fs::read_to_string(defines);
            if let Ok(spec) = spec {
                let spec = read_ab_with_apml(&spec)?;
                let ver = spec.get("VER");
                let rel = spec.get("REL");
                let defines = defines
                    .ok()
                    .and_then(|defines| read_ab_with_apml(&defines).ok());

                let epoch = if let Some(ref def) = defines {
                    def.get("PKGEPOCH")
                } else {
                    None
                };

                if ver.is_none() {
                    debug!("{pkg} has no VER variable");
                }

                let mut final_version = String::new();
                if let Some(epoch) = epoch {
                    final_version.push_str(&format!("{epoch}:"));
                }

                final_version.push_str(ver.unwrap());

                if let Some(rel) = rel {
                    final_version.push_str(&format!("-{rel}"));
                }

                res.push(format!("- {pkg}: {final_version}"));
            }
        }
    }

    Ok(res)
}

fn read_ab_with_apml(file: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut context = HashMap::new();

    // Try to set some ab3 flags to reduce the chance of returning errors
    for i in ["ARCH", "PKGDIR", "SRCDIR"] {
        context.insert(i.to_string(), "".to_string());
    }

    abbs_meta_apml::parse(file, &mut context).map_err(|e| {
        let e: Vec<String> = e.iter().map(|e| e.to_string()).collect();
        anyhow!(e.join(": "))
    })?;

    Ok(context)
}

fn handle_commits(commits: &[Commit]) -> anyhow::Result<String> {
    let mut s = String::new();
    for c in commits {
        s.push_str(&format!("- {}\n", c.msg.0));
        if let Some(body) = &c.msg.1 {
            let body = body.split('\n');
            for line in body {
                s.push_str(&format!("    {line}\n"));
            }
        }
    }

    Ok(s)
}

struct Commit {
    _id: String,
    msg: (String, Option<String>),
}

fn get_commits(path: &Path) -> anyhow::Result<Vec<Commit>> {
    let mut res = vec![];
    let repo = get_repo(&path)?;
    let commits = repo
        .head()?
        .try_into_peeled_id()?
        .ok_or(anyhow!("Failed to get peeled id"))?
        .ancestors()
        .all()?;

    let refrences = repo.references()?;
    let branch = refrences
        .local_branches()?
        .filter_map(Result::ok)
        .filter(|x| x.name().shorten() == "stable")
        .next()
        .ok_or(anyhow!("failed to get stable branch"))?;

    for i in commits {
        let o = i?.id.attach(&repo).object()?;
        let commit = o.into_commit();
        let commit_str = commit.id.to_string();

        if commit_in_stable(branch.clone(), &commit_str).unwrap_or(false) {
            break;
        }

        let msg = commit.message()?;

        res.push(Commit {
            _id: commit_str,
            msg: (msg.title.to_string(), msg.body.map(|x| x.to_string())),
        })
    }

    Ok(res)
}

fn commit_in_stable(branch: gix::Reference<'_>, commit: &str) -> anyhow::Result<bool> {
    let res = branch
        .into_fully_peeled_id()?
        .object()?
        .into_commit()
        .id
        .to_string()
        == commit;

    Ok(res)
}

/// Open Pull Request
async fn open_pr_inner(
    access_token: String,
    parts: &[&str],
    id: u64,
    key: EncodingKey,
    desc: &str,
    pkg_affected: &[String],
    tags: Option<&[String]>,
) -> Result<PullRequest, octocrab::Error> {
    let crab = octocrab::Octocrab::builder()
        .app(id.into(), key)
        .user_access_token(access_token)
        .build()?;

    let pr = crab
        .pulls("AOSC-Dev", "aosc-os-abbs")
        .create(parts[0], parts[1], "stable")
        .draft(false)
        .maintainer_can_modify(true)
        .body(format!(
            PR!(),
            desc,
            pkg_affected.join("\n"),
            format!("#buildit {}", parts[2].replace(",", " "))
        ))
        .send()
        .await?;

    let tags = if let Some(tags) = tags {
        Cow::Borrowed(tags)
    } else {
        let mut labels = vec![];
        let title = parts[0]
            .to_ascii_lowercase()
            .split_ascii_whitespace()
            .map(|x| {
                x.chars()
                    .filter(|x| x.is_ascii_alphabetic() || x.is_ascii_alphanumeric())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ");

        let v = vec![
            ("fix", vec![String::from("has-fix")]),
            ("update", vec![String::from("upgrade")]),
            ("upgrade", vec![String::from("upgrade")]),
            ("downgrade", vec![String::from("downgrade")]),
            ("survey", vec![String::from("survey")]),
            ("drop", vec![String::from("drop-package")]),
            ("security", vec![String::from("security")]),
            ("cve", vec![String::from("security")]),
            ("0day", vec![String::from("0day"), String::from("security")]),
            ("improve", vec![String::from("enhancement")]),
            ("enhance", vec![String::from("enhancement")]),
            ("dep", vec![String::from("dependencies")]),
            ("dependencies", vec![String::from("dependencies")]),
            ("dependency", vec![String::from("dependencies")]),
            ("pkgdep", vec![String::from("dependencies")]),
            ("builddep", vec![String::from("dependencies")]),
            ("depend", vec![String::from("dependencies")]),
            ("core", vec![String::from("core")]),
            ("mips64r6el", vec![String::from("cip-pilot")]),
            ("mipsisa64r6el", vec![String::from("cip-pilot")]),
            ("mipsr6", vec![String::from("cip-pilot")]),
            ("r6", vec![String::from("cip-pilot")]),
            ("linux-kernel", vec![String::from("kernel")]),
            ("new", vec![String::from("new-packages")]),
            (
                "ftbfs",
                vec![String::from("has-fix"), String::from("ftbfs")],
            ),
        ];

        for (k, v) in v {
            if title.contains(&k) {
                labels.extend(v);
            }
        }

        // de-duplicate
        let mut res = vec![];
        for i in labels {
            if res.contains(&i) {
                continue;
            }

            res.push(i);
        }

        Cow::Owned(res)
    };

    crab.issues("AOSC-Dev", "aosc-os-abbs")
        .add_labels(pr.number, &tags)
        .await?;

    Ok(pr)
}

/// Update ABBS tree commit logs
async fn update_abbs(git_ref: &str) -> anyhow::Result<()> {
    let abbs_path = ARGS
        .abbs_path
        .as_ref()
        .ok_or_else(|| anyhow!("ABBS_PATH is not set"))?;

    process::Command::new("git")
        .arg("checkout")
        .arg("-b")
        .arg("stable")
        .current_dir(abbs_path)
        .output()
        .await?;

    let cmd = process::Command::new("git")
        .arg("checkout")
        .arg("stable")
        .current_dir(abbs_path)
        .output()
        .await?;

    if !cmd.status.success() {
        bail!("Failed to checkout stable");
    }

    process::Command::new("git")
        .arg("pull")
        .current_dir(abbs_path)
        .output()
        .await?;

    process::Command::new("git")
        .args(&["reset", "FETCH_HEAD", "--hard"])
        .current_dir(abbs_path)
        .output()
        .await?;

    let cmd = process::Command::new("git")
        .arg("fetch")
        .arg("origin")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .await?;

    if !cmd.status.success() {
        bail!("Failed to fetch origin");
    }

    process::Command::new("git")
        .arg("checkout")
        .arg("-b")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .await?;

    let cmd = process::Command::new("git")
        .arg("checkout")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .await?;

    if !cmd.status.success() {
        bail!("Failed to checkout {git_ref}");
    }

    process::Command::new("git")
        .args(&["reset", "FETCH_HEAD", "--hard"])
        .current_dir(abbs_path)
        .output()
        .await?;

    Ok(())
}

fn get_repo(path: &Path) -> anyhow::Result<Repository> {
    let mut git_open_opts_map = sec::trust::Mapping::<gix::open::Options>::default();

    let config = gix::open::permissions::Config {
        git_binary: false,
        system: false,
        git: false,
        user: false,
        env: true,
        includes: true,
    };

    git_open_opts_map.reduced = git_open_opts_map
        .reduced
        .permissions(gix::open::Permissions {
            config,
            ..gix::open::Permissions::default_for_level(sec::Trust::Reduced)
        });

    git_open_opts_map.full = git_open_opts_map.full.permissions(gix::open::Permissions {
        config,
        ..gix::open::Permissions::default_for_level(sec::Trust::Full)
    });

    let shared_repo = ThreadSafeRepository::discover_with_environment_overrides_opts(
        path,
        Default::default(),
        git_open_opts_map,
    )
    .context("Failed to find git repo")?;

    let repository = shared_repo.to_thread_local();

    Ok(repository)
}
