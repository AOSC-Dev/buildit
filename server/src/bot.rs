use crate::{
    github::{get_github_token, login_github, open_pr},
    utils::all_packages_is_noarch,
    Args, ALL_ARCH, ARGS, WORKERS,
};
use chrono::Local;
use common::{ensure_job_queue, Job};
use lapin::{options::BasicPublishOptions, BasicProperties, ConnectionProperties};
use log::info;
use teloxide::{
    prelude::*,
    types::{ChatAction, ParseMode},
    utils::command::BotCommands,
};

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "BuildIt! supports the following commands:"
)]
pub enum Command {
    #[command(description = "Display usage: /help")]
    Help,
    #[command(
        description = "Start a build job: /build git-ref packages archs (e.g., /build stable bash,fish amd64,arm64)"
    )]
    Build(String),
    #[command(description = "Start a build job from GitHub PR: /pr pr-number [architectures]")]
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
            arch: if arch == &"noarch" {
                "amd64".to_string()
            } else {
                arch.to_string()
            },
            tg_chatid: msg.chat.id,
            github_pr,
            noarch: arch == &"noarch",
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
        archs.extend_from_slice(ALL_ARCH);
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
    for arch in ALL_ARCH {
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
            let parts = arguments.split_ascii_whitespace().collect::<Vec<_>>();
            if !(1..=2).contains(&parts.len()) {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Got invalid job description: {arguments}. \n\n{}",
                        Command::descriptions().to_string()
                    ),
                )
                .await?;
            }

            if let Ok(pr_number) = str::parse::<u64>(&parts[0]) {
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
                            let archs = if parts.len() == 1 {
                                let path = &ARGS.abbs_path;
                                let p = match path {
                                    Some(p) => p,
                                    None => {
                                        bot.send_message(
                                            msg.chat.id,
                                            "Got Error: ABBS_PATH is not set",
                                        )
                                        .await?;
                                        return Ok(());
                                    }
                                };
                                if all_packages_is_noarch(&packages, p).unwrap_or(false) {
                                    vec!["noarch"]
                                } else {
                                    // FIXME: loongarch64 is not in the mainline yet and should not be compiled automatically
                                    // let v = ALL_ARCH.to_vec();
                                    ALL_ARCH
                                        .iter()
                                        .filter(|x| x != &&"loongarch64")
                                        .map(|x| x.to_owned())
                                        .collect()
                                }
                            } else {
                                parts[1].split(',').collect()
                            };

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
            let (title, mut parts) = split_open_pr_message(&arguments);

            if let Some(title) = title {
                parts.insert(0, title);
            } else {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Got invalid job description: {arguments}. \n\n{}",
                        Command::descriptions().to_string()
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
                    if parts[3] == "" {
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

                let archs = if parts.len() == 5 {
                    parts[4].split(',').collect::<Vec<_>>()
                } else {
                    ALL_ARCH.to_vec()
                };

                match open_pr(parts, token, secret, msg.chat.id, tags.as_deref(), &archs).await {
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
                let resp = login_github(&msg, arguments).await;

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

fn split_open_pr_message(arguments: &str) -> (Option<&str>, Vec<&str>) {
    let mut parts = arguments.split(";");
    let title = parts.next();
    let parts = parts.map(|x| x.trim()).collect::<Vec<_>>();

    (title, parts)
}

#[test]
fn test_split_open_pr_message() {
    let t = split_open_pr_message("clutter fix ftbfs;clutter-fix-ftbfs;clutter");
    assert_eq!(t, (Some("clutter fix ftbfs"), vec!["clutter-fix-ftbfs", "clutter"]));

    let t = split_open_pr_message("clutter fix ftbfs;clutter-fix-ftbfs ;clutter");
    assert_eq!(t, (Some("clutter fix ftbfs"), vec!["clutter-fix-ftbfs", "clutter"]));
}
