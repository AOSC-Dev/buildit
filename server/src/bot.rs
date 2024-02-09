use std::{borrow::Cow, sync::Arc};

use crate::{
    formatter::{code_repr_string, to_html_new_job_summary},
    github::{get_github_token, get_packages_from_pr, login_github},
    job::{get_ready_message, send_build_request},
    Args, ALL_ARCH, ARGS, WORKERS,
};
use buildit_utils::github::{get_archs, update_abbs, OpenPRError, OpenPRRequest};
use chrono::Local;
use common::{ensure_job_queue, JobSource};
use lapin::{Channel, ConnectionProperties};
use serde_json::Value;
use teloxide::{
    prelude::*,
    types::{ChatAction, ParseMode},
    utils::command::BotCommands,
};
use tokio::process;

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "BuildIt! supports the following commands:"
)]
pub enum Command {
    #[command(description = "Display usage: /help")]
    Help,
    #[command(
        description = "Start a build job: /build branch packages archs (e.g., /build stable bash,fish amd64,arm64)"
    )]
    Build(String),
    #[command(
        description = "Start one or more build jobs from GitHub PR: /pr pr-numbers [archs] (e.g., /pr 12,34 amd64,arm64)"
    )]
    PR(String),
    #[command(description = "Show queue and server status: /status")]
    Status,
    #[command(
        description = "Open Pull Request by git-ref: /openpr title;git-ref;packages;[labels];[architectures] (e.g., /openpr VSCode Survey 1.85.0;vscode-1.85.0;vscode,vscodium;;amd64,arm64"
    )]
    OpenPR(String),
    #[command(description = "Login to github")]
    Login,
    #[command(description = "Start bot")]
    Start(String),
    #[command(description = "Queue all ready messages: /queue [archs]")]
    Queue(String),
}

pub struct BuildRequest<'a> {
    pub branch: &'a str,
    pub packages: &'a [String],
    pub archs: &'a [&'a str],
    pub github_pr: Option<u64>,
    pub sha: &'a str,
}

async fn telegram_send_build_request(
    bot: &Bot,
    build_request: BuildRequest<'_>,
    msg: &Message,
    channel: &Channel,
) -> ResponseResult<()> {
    let BuildRequest {
        branch,
        packages,
        archs,
        github_pr,
        sha,
    } = build_request;

    let archs = handle_archs_args(archs.to_vec());

    match send_build_request(
        branch,
        packages,
        &archs,
        github_pr,
        JobSource::Telegram(msg.chat.id.0),
        sha,
        channel,
    )
    .await
    {
        Ok(()) => {
            bot.send_message(
                msg.chat.id,
                to_html_new_job_summary(branch, github_pr, &archs, packages),
            )
            .parse_mode(ParseMode::Html)
            .disable_web_page_preview(true)
            .await?;
        }
        Err(err) => {
            bot.send_message(msg.chat.id, format!("Failed to create job: {}", err))
                .await?;
        }
    }
    Ok(())
}

fn handle_archs_args(archs: Vec<&str>) -> Vec<&str> {
    let mut archs = archs;
    if archs.contains(&"mainline") {
        // archs
        archs.extend(ALL_ARCH.iter());
        archs.retain(|arch| *arch != "mainline");
    }
    archs.sort();
    archs.dedup();

    archs
}

async fn status(args: &Args) -> anyhow::Result<String> {
    let mut res = String::from("__*Queue Status*__\n\n");
    let conn = lapin::Connection::connect(&ARGS.amqp_addr, ConnectionProperties::default()).await?;
    let channel = conn.create_channel().await?;

    for arch in ALL_ARCH {
        let queue_name = format!("job-{}", arch);

        let queue = ensure_job_queue(&queue_name, &channel).await?;

        // read unacknowledged job count
        let mut unacknowledged_str = String::new();
        if let Some(api) = &args.rabbitmq_queue_api {
            let res = http_rabbitmq_api(api, queue_name).await?;
            if let Some(unacknowledged) = res
                .as_object()
                .and_then(|m| m.get("messages_unacknowledged"))
                .and_then(|v| v.as_i64())
            {
                unacknowledged_str = format!("{} job\\(s\\) running, ", unacknowledged);
            }
        }
        res += &format!(
            "*{}*: {}{} jobs\\(s\\) pending, {} available server\\(s\\)\n",
            teloxide::utils::markdown::escape(arch),
            unacknowledged_str,
            queue.message_count(),
            queue.consumer_count()
        );
    }

    res += "\n__*Server Status*__\n\n";
    let fmt = timeago::Formatter::new();
    if let Ok(lock) = WORKERS.lock() {
        for (identifier, status) in lock.iter() {
            res += &teloxide::utils::markdown::escape(&format!(
                "{} ({}{}): Online as of {}\n",
                identifier.hostname,
                identifier.arch,
                match &status.git_commit {
                    Some(git_commit) => format!(" {}", git_commit),
                    None => String::new(),
                },
                fmt.convert_chrono(status.last_heartbeat, Local::now())
            ));
        }
    }
    Ok(res)
}

pub async fn http_rabbitmq_api(api: &str, queue_name: String) -> anyhow::Result<Value> {
    let client = reqwest::Client::new();

    let res = client
        .get(format!("{}{}", api, queue_name))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    Ok(res)
}

pub async fn answer(
    bot: Bot,
    msg: Message,
    cmd: Command,
    channel: Arc<Channel>,
) -> ResponseResult<()> {
    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::PR(arguments) => {
            let parts = arguments.split_ascii_whitespace().collect::<Vec<_>>();
            if !(1..=2).contains(&parts.len()) {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Got invalid job description: {arguments}. \n\n{}",
                        Command::descriptions()
                    ),
                )
                .await?;
            }

            let mut pr_numbers = vec![];
            let mut parse_success = true;
            for part in parts[0].split(',') {
                if let Ok(pr_number) = str::parse::<u64>(part) {
                    pr_numbers.push(pr_number);
                } else {
                    parse_success = false;

                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "Got invalid pr description: {arguments}.\n\n{}",
                            Command::descriptions()
                        ),
                    )
                    .await?;
                    break;
                }
            }

            if parse_success {
                for pr_number in pr_numbers {
                    match octocrab::instance()
                        .pulls("AOSC-Dev", "aosc-os-abbs")
                        .get(pr_number)
                        .await
                    {
                        Ok(pr) => {
                            // If the pull request has been merged,
                            // build and push packages based on stable
                            let (branch, sha) = if pr.merged_at.is_some() {
                                (
                                    "stable",
                                    pr.merge_commit_sha
                                        .as_ref()
                                        .expect("merge_commit_sha should not be None"),
                                )
                            } else {
                                (pr.head.ref_field.as_str(), &pr.head.sha)
                            };

                            if pr.head.repo.as_ref().and_then(|x| x.fork).unwrap_or(false) {
                                bot.send_message(
                                    msg.chat.id,
                                    "Failed to create job: Pull request is a fork",
                                )
                                .await?;
                                return Ok(());
                            }

                            let path = &ARGS.abbs_path;

                            if let Err(e) = update_abbs(branch, path).await {
                                bot.send_message(msg.chat.id, e.to_string()).await?;
                            }

                            // find lines starting with #buildit
                            let packages = get_packages_from_pr(&pr);
                            if !packages.is_empty() {
                                let archs = if parts.len() == 1 {
                                    let path = &ARGS.abbs_path;

                                    get_archs(path, &packages)
                                } else {
                                    let archs = parts[1].split(',').collect();

                                    for a in &archs {
                                        if !ALL_ARCH.contains(a) && a != &"mainline" {
                                            bot.send_message(
                                                msg.chat.id,
                                                format!("Architecture {a} is not supported."),
                                            )
                                            .await?;
                                            return Ok(());
                                        }
                                    }

                                    archs
                                };

                                let build_request = BuildRequest {
                                    branch,
                                    packages: &packages,
                                    archs: &archs,
                                    github_pr: Some(pr_number),
                                    sha,
                                };

                                telegram_send_build_request(&bot, build_request, &msg, &channel)
                                    .await?;
                            } else {
                                bot.send_message(msg.chat.id, "Please list packages to build in pr info starting with '#buildit'.".to_string())
                                .await?;
                            }
                        }
                        Err(err) => {
                            bot_send_message_handle_length(
                                &bot,
                                &msg,
                                &format!("Failed to get pr info: {err}."),
                            )
                            .await?;
                        }
                    }
                }
            }
        }
        Command::Build(arguments) => {
            let parts: Vec<&str> = arguments.split(' ').collect();
            if parts.len() == 3 {
                let branch = parts[0];
                let packages: Vec<String> = parts[1].split(',').map(str::to_string).collect();
                let archs: Vec<&str> = parts[2].split(',').collect();

                // resolve branch name to commit hash
                let path = &ARGS.abbs_path;

                if let Err(e) = update_abbs(branch, path).await {
                    bot.send_message(msg.chat.id, format!("Failed to update ABBS tree: {e}"))
                        .await?;
                } else {
                    let output = process::Command::new("git")
                        .arg("rev-parse")
                        .arg("HEAD")
                        .current_dir(path)
                        .output()
                        .await?;
                    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();

                    let build_request = BuildRequest {
                        branch,
                        packages: &packages,
                        archs: &archs,
                        github_pr: None,
                        sha: &sha,
                    };
                    telegram_send_build_request(&bot, build_request, &msg, &channel).await?;
                }
                return Ok(());
            }

            bot.send_message(
                msg.chat.id,
                format!(
                    "Got invalid job description: {arguments}. \n\n{}",
                    Command::descriptions()
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
            let (title, mut parts) = split_open_pr_message(&arguments);

            if let Some(title) = title {
                parts.insert(0, title);
            } else {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Got invalid job description: {arguments}. \n\n{}",
                        Command::descriptions()
                    ),
                )
                .await?;
                return Ok(());
            }

            let secret = match ARGS.secret.as_ref() {
                Some(s) => s,
                None => {
                    bot.send_message(msg.chat.id, "SECRET is not set").await?;
                    return Ok(());
                }
            };

            let token = match get_github_token(&msg.chat.id, secret).await {
                Ok(s) => s.access_token,
                Err(e) => {
                    bot.send_message(msg.chat.id, format!("Got error: {e}"))
                        .await?;
                    return Ok(());
                }
            };

            if (3..=5).contains(&parts.len()) {
                let tags = if parts.len() >= 4 {
                    if parts[3].is_empty() {
                        None
                    } else {
                        Some(
                            parts[3]
                                .split(',')
                                .map(|x| x.to_string())
                                .collect::<Vec<_>>(),
                        )
                    }
                } else {
                    None
                };

                let path = ARGS.abbs_path.as_ref();

                let pkgs = parts[2]
                    .split(',')
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>();

                let archs = if parts.len() == 5 {
                    let archs = parts[4].split(',').collect::<Vec<_>>();
                    handle_archs_args(archs)
                } else {
                    get_archs(path, &pkgs)
                };

                let id = match ARGS
                    .github_app_id
                    .as_ref()
                    .and_then(|x| x.parse::<u64>().ok())
                {
                    Some(id) => id,
                    None => {
                        bot.send_message(msg.chat.id, "Got Error: GITHUB_APP_ID is not set")
                            .await?;
                        return Ok(());
                    }
                };

                let app_private_key = match ARGS.github_app_key.as_ref() {
                    Some(p) => p,
                    None => {
                        bot.send_message(msg.chat.id, "Got Error: GITHUB_APP_ID is not set")
                            .await?;
                        return Ok(());
                    }
                };

                match buildit_utils::github::open_pr(
                    app_private_key,
                    &token,
                    id,
                    OpenPRRequest {
                        git_ref: parts[1].to_owned(),
                        abbs_path: ARGS.abbs_path.clone(),
                        packages: parts[2].to_owned(),
                        title: parts[0].to_string(),
                        tags: tags.clone(),
                        archs: archs.clone(),
                    },
                )
                .await
                {
                    Ok(url) => {
                        bot.send_message(msg.chat.id, format!("Successfully opened PR: {url}"))
                            .await?;
                        return Ok(());
                    }
                    Err(e) => match e {
                        OpenPRError::Github(e) => match e {
                            octocrab::Error::GitHub { source, .. }
                                if source.message.contains("Bad credentials") =>
                            {
                                let client = reqwest::Client::new();
                                client
                                    .get("https://minzhengbu.aosc.io/refresh_token")
                                    .header("secret", secret)
                                    .query(&[("id", msg.chat.id.0.to_string())])
                                    .send()
                                    .await
                                    .and_then(|x| x.error_for_status())?;

                                let token = match get_github_token(&msg.chat.id, secret).await {
                                    Ok(s) => s.access_token,
                                    Err(e) => {
                                        bot.send_message(msg.chat.id, format!("Got error: {e}"))
                                            .await?;
                                        return Ok(());
                                    }
                                };

                                match buildit_utils::github::open_pr(
                                    app_private_key,
                                    &token,
                                    id,
                                    OpenPRRequest {
                                        git_ref: parts[1].to_owned(),
                                        abbs_path: ARGS.abbs_path.clone(),
                                        packages: parts[2].to_owned(),
                                        title: parts[0].to_string(),
                                        tags,
                                        archs,
                                    },
                                )
                                .await
                                {
                                    Ok(url) => {
                                        bot.send_message(
                                            msg.chat.id,
                                            format!("Successfully opened PR: {url}"),
                                        )
                                        .await?;
                                        return Ok(());
                                    }
                                    Err(e) => {
                                        bot_send_message_handle_length(&bot, &msg, &format!("{e}"))
                                            .await?;
                                        return Ok(());
                                    }
                                }
                            }
                            _ => {
                                bot_send_message_handle_length(&bot, &msg, &format!("{e}")).await?;
                                return Ok(());
                            }
                        },
                        _ => {
                            bot_send_message_handle_length(&bot, &msg, &format!("{e}")).await?;
                            return Ok(());
                        }
                    },
                }
            }

            bot.send_message(
                msg.chat.id,
                format!(
                    "Got invalid job description: {arguments}. \n\n{}",
                    Command::descriptions()
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
                let resp = login_github(&msg, arguments).await;

                match resp {
                    Ok(_) => bot.send_message(msg.chat.id, "Login successful!").await?,
                    Err(e) => {
                        bot_send_message_handle_length(
                            &bot,
                            &msg,
                            &format!("Login failed with error: {e}"),
                        )
                        .await?
                    }
                };
            }
        }
        Command::Queue(arguments) => {
            let mut archs = vec![];
            if !arguments.is_empty() {
                archs.extend(arguments.split(','));
            } else {
                archs.extend(ALL_ARCH);
            }

            match get_ready_message(&ARGS.amqp_addr, &archs).await {
                Ok(map) => {
                    let mut res = String::new();
                    for (k, v) in map {
                        res.push_str(&format!("{k}:\n"));
                        res.push_str(&format!("{}\n", code_repr_string(&v)));
                    }

                    if res.is_empty() {
                        bot.send_message(msg.chat.id, "Queue is empty").await?;
                    } else {
                        bot.send_message(msg.chat.id, res)
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                }
                Err(e) => {
                    bot.send_message(msg.chat.id, e.to_string()).await?;
                }
            }
        }
    };

    Ok(())
}

async fn bot_send_message_handle_length(
    bot: &Bot,
    msg: &Message,
    text: &str,
) -> Result<Message, teloxide::RequestError> {
    let text = if text.chars().count() > 1000 {
        console::truncate_str(text, 1000, "...")
    } else {
        Cow::Borrowed(text)
    };

    bot.send_message(msg.chat.id, text).await
}

fn split_open_pr_message(arguments: &str) -> (Option<&str>, Vec<&str>) {
    let mut parts = arguments.split(';');
    let title = parts.next();
    let parts = parts.map(|x| x.trim()).collect::<Vec<_>>();

    (title, parts)
}

#[test]
fn test_split_open_pr_message() {
    let t = split_open_pr_message("clutter fix ftbfs;clutter-fix-ftbfs;clutter");
    assert_eq!(
        t,
        (
            Some("clutter fix ftbfs"),
            vec!["clutter-fix-ftbfs", "clutter"]
        )
    );

    let t = split_open_pr_message("clutter fix ftbfs;clutter-fix-ftbfs ;clutter");
    assert_eq!(
        t,
        (
            Some("clutter fix ftbfs"),
            vec!["clutter-fix-ftbfs", "clutter"]
        )
    );
}
