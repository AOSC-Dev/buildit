use crate::{Args, ARGS, WORKERS};
use anyhow::{anyhow, bail, Context};
use chrono::Local;
use common::{ensure_job_queue, Job};
use gix::{
    prelude::ObjectIdExt,
    sec::{self, trust::DefaultForLevel},
    Repository, ThreadSafeRepository,
};
use jsonwebtoken::EncodingKey;
use lapin::{options::BasicPublishOptions, BasicProperties, ConnectionProperties};
use log::{debug, info};
use octocrab::models::pulls::PullRequest;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap, path::Path};
use teloxide::{
    prelude::*,
    types::{ChatAction, ParseMode},
    utils::command::BotCommands,
};
use tokio::{process, task};
use walkdir::WalkDir;

macro_rules! PR {
    () => {
        "Topic Description\n-----------------\n\n{}Package(s) Affected\n-------------------\n\n{}\n\nSecurity Update?\n----------------\n\nNo\n\nBuild Order\n-----------\n\n```\n{}\n```\n\nTest Build(s) Done\n------------------\n\n**Primary Architectures**\n\n- [ ] AMD64 `amd64`   \n- [ ] AArch64 `arm64`\n \n<!-- - [ ] 32-bit Optional Environment `optenv32` -->\n<!-- - [ ] Architecture-independent `noarch` -->\n\n**Secondary Architectures**\n\n- [ ] Loongson 3 `loongson3`\n- [ ] MIPS R6 64-bit (Little Endian) `mips64r6el`\n- [ ] PowerPC 64-bit (Little Endian) `ppc64el`\n- [ ] RISC-V 64-bit `riscv64`"
    };
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "BuildIt! supports the following commands:"
)]
pub enum Command {
    #[command(description = "Display usage: /help")]
    Help,
    #[command(
        description = "Start a build job: /build [git-ref] [packages] [archs] (e.g., /build stable bash,fish amd64,arm64)"
    )]
    Build(String),
    #[command(description = "Start a build job from GitHub PR: /pr [pr-number]")]
    PR(String),
    #[command(description = "Show queue and server status: /status")]
    Status,
    #[command(
        description = "Open Pull Request by git-ref: /openpr [title];[git-ref];[packages] (e.g., /openpr VSCode Survey 1.85.0;vscode-1.85.0;vscode,vscodium"
    )]
    OpenPR(String),
    #[command(description = "Login to github")]
    Login,
    #[command(description = "Start bot")]
    Start(String),
}

async fn build_inner(
    git_ref: &str,
    packages: &Vec<String>,
    archs: &Vec<&str>,
    github_pr: Option<u64>,
    msg: &Message,
) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(&ARGS.amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    // for each arch, create a job
    for arch in archs {
        let job = Job {
            packages: packages.iter().map(|s| s.to_string()).collect(),
            git_ref: git_ref.to_string(),
            arch: arch.to_string(),
            tg_chatid: msg.chat.id,
            github_pr,
        };

        info!("Adding job to message queue {:?} ...", job);

        // each arch has its own queue
        let queue_name = format!("job-{}", job.arch);
        ensure_job_queue(&queue_name, &channel).await?;

        channel
            .basic_publish(
                "",
                &queue_name,
                BasicPublishOptions::default(),
                &serde_json::to_vec(&job)?,
                BasicProperties::default(),
            )
            .await?
            .await?;
    }
    Ok(())
}

async fn build(
    bot: &Bot,
    git_ref: &str,
    packages: &Vec<String>,
    archs: &Vec<&str>,
    github_pr: Option<u64>,
    msg: &Message,
) -> ResponseResult<()> {
    let mut archs = archs.clone();
    if archs.contains(&"mainline") {
        // follow https://github.com/AOSC-Dev/autobuild3/blob/master/sets/arch_groups/mainline
        archs.extend_from_slice(&[
            "amd64",
            "arm64",
            "loongarch64",
            "loongson3",
            "mips64r6el",
            "ppc64el",
            "riscv64",
        ]);
        archs.retain(|arch| *arch != "mainline");
    }
    archs.sort();
    archs.dedup();

    match build_inner(git_ref, &packages, &archs, github_pr, &msg).await {
        Ok(()) => {
            bot.send_message(
                            msg.chat.id,
                            format!(
                                "\n__*New Job Summary*__\n\n*Git reference*: {}\n{}*Architecture\\(s\\)*: {}\n*Package\\(s\\)*: {}\n",
                                teloxide::utils::markdown::escape(git_ref),
                                if let Some(pr) = github_pr { format!("*GitHub PR*: [\\#{}](https://github.com/AOSC-Dev/aosc-os-abbs/pull/{})\n", pr, pr) } else { String::new() },
                                archs.join(", "),
                                teloxide::utils::markdown::escape(&packages.join(", ")),
                            ),
                        ).parse_mode(ParseMode::MarkdownV2)
                        .await?;
        }
        Err(err) => {
            bot.send_message(msg.chat.id, format!("Failed to create job: {}", err))
                .await?;
        }
    }
    Ok(())
}

async fn status(args: &Args) -> anyhow::Result<String> {
    let mut res = String::from("__*Queue Status*__\n\n");
    let conn = lapin::Connection::connect(&ARGS.amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    for arch in [
        "amd64",
        "arm64",
        "loongarch64",
        "loongson3",
        "mips64r6el",
        "ppc64el",
        "riscv64",
    ] {
        let queue_name = format!("job-{}", arch);

        let queue = ensure_job_queue(&queue_name, &channel).await?;

        // read unacknowledged job count
        let mut unacknowledged_str = String::new();
        if let Some(api) = &args.rabbitmq_queue_api {
            let client = reqwest::Client::new();
            let res = client
                .get(format!("{}{}", api, queue_name))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;
            if let Some(unacknowledged) = res
                .as_object()
                .and_then(|m| m.get("messages_unacknowledged"))
                .and_then(|v| v.as_i64())
            {
                unacknowledged_str = format!("{} job\\(s\\), ", unacknowledged);
            }
        }
        res += &format!(
            "*{}*: {}{} available server\\(s\\)\n",
            teloxide::utils::markdown::escape(&arch),
            unacknowledged_str,
            queue.consumer_count()
        );
    }

    res += "\n__*Server Status*__\n\n";
    let fmt = timeago::Formatter::new();
    if let Ok(lock) = WORKERS.lock() {
        for (identifier, status) in lock.iter() {
            res += &teloxide::utils::markdown::escape(&format!(
                "{} ({}): Online as of {}\n",
                identifier.hostname,
                identifier.arch,
                fmt.convert_chrono(status.last_heartbeat, Local::now())
            ));
        }
    }
    Ok(res)
}

pub async fn answer(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::PR(arguments) => {
            if let Ok(pr_number) = str::parse::<u64>(&arguments) {
                match octocrab::instance()
                    .pulls("AOSC-Dev", "aosc-os-abbs")
                    .get(pr_number)
                    .await
                {
                    Ok(pr) => {
                        let git_ref = &pr.head.ref_field;
                        // find lines starting with #buildit
                        let packages: Vec<String> = pr
                            .body
                            .and_then(|body| {
                                body.lines()
                                    .filter(|line| line.starts_with("#buildit"))
                                    .map(|line| {
                                        line.split(" ")
                                            .map(str::to_string)
                                            .skip(1)
                                            .collect::<Vec<_>>()
                                    })
                                    .next()
                            })
                            .unwrap_or_else(Vec::new);
                        if packages.len() > 0 {
                            let archs = vec![
                                "amd64",
                                "arm64",
                                "loongson3",
                                "mips64r6el",
                                "ppc64el",
                                "riscv64",
                            ];
                            build(&bot, git_ref, &packages, &archs, Some(pr_number), &msg).await?;
                        } else {
                            bot.send_message(msg.chat.id, format!("Please list packages to build in pr info starting with '#buildit'."))
                                .await?;
                        }
                    }
                    Err(err) => {
                        bot.send_message(msg.chat.id, format!("Failed to get pr info: {err}."))
                            .await?;
                    }
                }
            } else {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Got invalid pr description: {arguments}.\n\n{}",
                        Command::descriptions().to_string()
                    ),
                )
                .await?;
            }
        }
        Command::Build(arguments) => {
            let parts: Vec<&str> = arguments.split(" ").collect();
            if parts.len() == 3 {
                let git_ref = parts[0];
                let packages: Vec<String> = parts[1].split(",").map(str::to_string).collect();
                let archs: Vec<&str> = parts[2].split(",").collect();
                build(&bot, git_ref, &packages, &archs, None, &msg).await?;
                return Ok(());
            }

            bot.send_message(
                msg.chat.id,
                format!(
                    "Got invalid job description: {arguments}. \n\n{}",
                    Command::descriptions().to_string()
                ),
            )
            .await?;
        }
        Command::Status => match status(&ARGS).await {
            Ok(status) => {
                bot.send_message(msg.chat.id, status)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
            }
            Err(err) => {
                bot.send_message(msg.chat.id, format!("Failed to get status: {}", err))
                    .await?;
            }
        },
        Command::OpenPR(arguments) => {
            let parts: Vec<&str> = arguments.split(";").collect();

            let secret = match ARGS.secret.as_ref() {
                Some(s) => s,
                None => {
                    bot.send_message(msg.chat.id, "SECRET is not set").await?;
                    return Ok(());
                }
            };

            let token = match get_token(&msg.chat.id, secret).await {
                Ok(s) => s.access_token,
                Err(e) => {
                    bot.send_message(msg.chat.id, format!("Got error: {e}"))
                        .await?;
                    return Ok(());
                }
            };

            if (3..=4).contains(&parts.len()) {
                let tags = if parts.len() == 4 {
                    Some(
                        parts
                            .last()
                            .unwrap()
                            .split(',')
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                };

                match open_pr(parts, token, secret, msg.chat.id, tags.as_deref()).await {
                    Ok(url) => {
                        bot.send_message(msg.chat.id, format!("Successfully opened PR: {url}"))
                            .await?
                    }
                    Err(e) => {
                        bot.send_message(msg.chat.id, format!("Got error: {e}"))
                            .await?
                    }
                };

                return Ok(());
            }

            bot.send_message(
                msg.chat.id,
                format!(
                    "Got invalid job description: {arguments}. \n\n{}",
                    Command::descriptions().to_string()
                ),
            )
            .await?;
        }
        Command::Login => {
            bot.send_message(msg.chat.id, "https://github.com/login/oauth/authorize?client_id=Iv1.bf26f3e9dd7883ae&redirect_uri=https://minzhengbu.aosc.io/login").await?;
        }
        Command::Start(arguments) => {
            if arguments.len() != 20 {
                bot.send_message(msg.chat.id, Command::descriptions().to_string())
                    .await?;
                return Ok(());
            } else {
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

                match resp {
                    Ok(_) => {
                        bot.send_message(msg.chat.id, "Successful to login.")
                            .await?
                    }
                    Err(e) => {
                        bot.send_message(msg.chat.id, format!("Got error: {e}"))
                            .await?
                    }
                };
            }
        }
    };

    Ok(())
}

#[derive(Deserialize, Serialize, Debug)]
struct CallbackSecondLoginArgs {
    access_token: String,
    expires_in: i64,
    refresh_token: String,
    refresh_token_expires_in: i64,
    scope: String,
    token_type: String,
}

async fn get_token(msg_chatid: &ChatId, secret: &str) -> anyhow::Result<CallbackSecondLoginArgs> {
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

async fn open_pr(
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

                let token = get_token(&msg_chatid, secret).await?;
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
            .chars()
            .filter(|x| x.is_ascii_alphabetic() || x.is_ascii_alphanumeric())
            .collect::<String>();
        let title = title.split_ascii_whitespace().collect::<Vec<_>>();

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
        ];

        for (k, v) in v {
            if title.contains(&k) {
                labels.extend(v);
            }
        }

        Cow::Owned(labels)
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
