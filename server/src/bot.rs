use crate::{
    api::{job_restart, pipeline_new, pipeline_new_pr, pipeline_status, worker_status, JobSource},
    formatter::to_html_new_pipeline_summary,
    github::{get_github_token, login_github},
    models::{NewUser, User},
    DbPool, ALL_ARCH, ARGS,
};
use anyhow::{bail, Context};
use buildit_utils::{find_update_and_update_checksum, github::OpenPRRequest};
use chrono::Local;
use diesel::{Connection, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use rand::prelude::SliceRandom;
use rand::thread_rng;
use reqwest::ClientBuilder;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fmt::Display};
use teloxide::{
    prelude::*,
    types::{ChatAction, ParseMode},
    utils::command::BotCommands,
};
use tracing::{warn, Instrument};

#[derive(BotCommands, Clone, Debug)]
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
    #[command(description = "Let dickens generate report for GitHub PR: /dickens pr-number")]
    Dickens(String),
    #[command(
        description = "Build lagging/missing packages for quality assurance: /qa arch lagging/missing"
    )]
    QA(String),
    #[command(description = "Restart failed job: /restart job-id")]
    Restart(String),
    #[command(description = "Find update and bump package version: /bump package-name")]
    Bump(String),
    #[command(description = "Roll anicca 10 packages")]
    Roll,
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

#[tracing::instrument(skip(pool))]
async fn status(pool: DbPool) -> anyhow::Result<String> {
    let mut res = String::from("__*Queue Status*__\n\n");

    for status in pipeline_status(pool.clone()).await? {
        res += &format!(
            "*{}*: {} job\\(s\\) pending, {} job\\(s\\) running, {} available server\\(s\\)\n",
            teloxide::utils::markdown::escape(&status.arch),
            status.pending,
            status.running,
            status.available_servers
        );
    }

    res += "\n__*Server Status*__\n\n";
    let fmt = timeago::Formatter::new();
    for status in worker_status(pool).await? {
        res += &teloxide::utils::markdown::escape(&format!(
            "{} ({} {}, {} core(s), {} memory): Online as of {}\n",
            status.hostname,
            status.arch,
            status.git_commit,
            status.logical_cores,
            size::Size::from_bytes(status.memory_bytes),
            fmt.convert_chrono(status.last_heartbeat_time, Local::now())
        ));
    }
    Ok(res)
}

#[derive(Deserialize)]
pub struct QAResponsePackage {
    name: String,
}

#[derive(Deserialize)]
pub struct QAResponse {
    packages: Vec<QAResponsePackage>,
}

#[tracing::instrument(skip(bot, pool, msg))]
async fn pipeline_new_and_report(
    bot: &Bot,
    pool: DbPool,
    git_branch: &str,
    packages: &str,
    archs: &str,
    msg: &Message,
) -> ResponseResult<()> {
    match pipeline_new(
        pool,
        git_branch,
        None,
        None,
        packages,
        archs,
        &JobSource::Telegram(msg.chat.id.0),
    )
    .await
    {
        Ok(pipeline) => {
            bot.send_message(
                msg.chat.id,
                to_html_new_pipeline_summary(
                    pipeline.id,
                    &pipeline.git_branch,
                    &pipeline.git_sha,
                    pipeline.github_pr.map(|n| n as u64),
                    &pipeline.archs.split(',').collect::<Vec<_>>(),
                    &pipeline.packages.split(',').collect::<Vec<_>>(),
                ),
            )
            .parse_mode(ParseMode::Html)
            .disable_web_page_preview(true)
            .await?;
        }
        Err(err) => {
            bot.send_message(msg.chat.id, truncate(&format!("{err:?}")))
                .await?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: i64,
    pub email: Option<String>,
    pub avatar_url: String,
    pub name: String,
}

#[tracing::instrument(skip(pool, access_token))]
async fn sync_github_info_inner(
    pool: DbPool,
    telegram_chat: ChatId,
    access_token: String,
) -> anyhow::Result<()> {
    let crab = octocrab::Octocrab::builder()
        .user_access_token(access_token)
        .build()?;
    let author: GitHubUser = crab.get("/user", None::<&()>).await?;
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        use crate::schema::users::dsl::*;
        match users
            .filter(telegram_chat_id.eq(&telegram_chat.0))
            .first::<User>(conn)
            .optional()?
        {
            Some(user) => {
                diesel::update(users.find(user.id))
                    .set((
                        github_login.eq(author.login),
                        github_id.eq(author.id),
                        github_avatar_url.eq(author.avatar_url.to_string()),
                        github_email.eq(author.email),
                        github_name.eq(author.name),
                    ))
                    .execute(conn)?;
            }
            None => {
                let new_user = NewUser {
                    github_login: Some(author.login),
                    github_id: Some(author.id),
                    github_name: Some(author.name),
                    github_avatar_url: Some(author.avatar_url.to_string()),
                    github_email: author.email,
                    telegram_chat_id: Some(telegram_chat.0),
                };
                diesel::insert_into(crate::schema::users::table)
                    .values(&new_user)
                    .execute(conn)?;
            }
        }

        Ok(())
    })?;
    Ok(())
}

#[tracing::instrument(skip(pool, access_token))]
async fn sync_github_info(pool: DbPool, telegram_chat_id: ChatId, access_token: String) {
    if let Err(err) = sync_github_info_inner(pool, telegram_chat_id, access_token).await {
        warn!(
            "Failed to sync github info for telegram chat {}: {}",
            telegram_chat_id, err
        );
    }
}

#[tracing::instrument(skip(pool, access_token))]
async fn get_user(pool: DbPool, chat_id: ChatId, access_token: String) -> anyhow::Result<User> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    use crate::schema::users::dsl::*;
    if let Some(user) = users
        .filter(telegram_chat_id.eq(&chat_id.0))
        .first::<User>(&mut conn)
        .optional()?
    {
        return Ok(user);
    }

    // not found, try to fetch user info
    sync_github_info_inner(pool, chat_id, access_token).await?;

    // try again
    if let Some(user) = users
        .filter(telegram_chat_id.eq(&chat_id.0))
        .first::<User>(&mut conn)
        .optional()?
    {
        return Ok(user);
    }

    bail!("Failed to get user info")
}

async fn create_pipeline_from_pr(
    pool: DbPool,
    pr_number: u64,
    archs: Option<&str>,
    msg: &Message,
    bot: &Bot,
) -> ResponseResult<()> {
    match pipeline_new_pr(pool, pr_number, archs, &JobSource::Telegram(msg.chat.id.0)).await {
        Ok(pipeline) => {
            bot.send_message(
                msg.chat.id,
                to_html_new_pipeline_summary(
                    pipeline.id,
                    &pipeline.git_branch,
                    &pipeline.git_sha,
                    pipeline.github_pr.map(|n| n as u64),
                    &pipeline.archs.split(',').collect::<Vec<_>>(),
                    &pipeline.packages.split(',').collect::<Vec<_>>(),
                ),
            )
            .parse_mode(ParseMode::Html)
            .disable_web_page_preview(true)
            .send()
            .instrument(tracing::info_span!("send_message"))
            .await?;
        }
        Err(err) => {
            bot.send_message(
                msg.chat.id,
                truncate(&format!("Failed to create pipeline from pr: {err:?}")),
            )
            .await?;
        }
    }

    Ok(())
}

#[tracing::instrument(skip(bot, msg, pool))]
pub async fn answer(bot: Bot, msg: Message, cmd: Command, pool: DbPool) -> ResponseResult<()> {
    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .send()
        .instrument(tracing::info_span!("send_chat_action"))
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
                return Ok(());
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
                let archs = if parts.len() == 1 {
                    None
                } else {
                    Some(parts[1])
                };
                for pr_number in pr_numbers {
                    create_pipeline_from_pr(pool.clone(), pr_number, archs, &msg, &bot).await?;
                }
            }
        }
        Command::Build(arguments) => {
            let parts: Vec<&str> = arguments.split(' ').collect();
            if parts.len() == 3 {
                let git_branch = parts[0];
                let packages = parts[1];
                let archs = parts[2];

                pipeline_new_and_report(&bot, pool, git_branch, packages, archs, &msg).await?;

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
        Command::Status => match status(pool).await {
            Ok(status) => {
                bot.send_message(msg.chat.id, status)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    truncate(&format!("Failed to get status: {:?}", err)),
                )
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

            let secret = match ARGS.github_secret.as_ref() {
                Some(s) => s,
                None => {
                    bot.send_message(msg.chat.id, "GITHUB_SECRET is not set")
                        .await?;
                    return Ok(());
                }
            };

            let token = match get_github_token(&msg.chat.id, secret).await {
                Ok(s) => s.access_token,
                Err(e) => {
                    bot.send_message(msg.chat.id, truncate(&format!("Got error: {e:?}")))
                        .await?;
                    return Ok(());
                }
            };

            // sync github info, but do not wait for result
            tokio::spawn(sync_github_info(pool, msg.chat.id, token.clone()));

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

                let archs = if parts.len() == 5 {
                    let archs = parts[4].split(',').collect::<Vec<_>>();
                    Some(handle_archs_args(archs))
                } else {
                    // deduce archs later
                    None
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
                    Ok((_id, url)) => {
                        bot.send_message(msg.chat.id, format!("Successfully opened PR: {url}"))
                            .await?;
                        return Ok(());
                    }
                    Err(e) => {
                        bot.send_message(msg.chat.id, truncate(&format!("Failed to open pr: {e}")))
                            .await?;
                        return Ok(());
                    }
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
                        bot.send_message(
                            msg.chat.id,
                            truncate(&format!("Login failed with error: {e}")),
                        )
                        .await?
                    }
                };
            }
        }
        Command::Dickens(arguments) => match str::parse::<u64>(&arguments) {
            Ok(pr_number) => {
                // create octocrab instance
                let crab = match octocrab::Octocrab::builder()
                    .user_access_token(ARGS.github_access_token.clone())
                    .build()
                {
                    Ok(v) => v,
                    Err(err) => {
                        bot.send_message(
                            msg.chat.id,
                            truncate(&format!("Cannot create octocrab instance: {err:?}")),
                        )
                        .await?;
                        return Ok(());
                    }
                };

                // get topic of pr
                match crab.pulls("AOSC-Dev", "aosc-os-abbs").get(pr_number).await {
                    Ok(pr) => match dickens::topic::report(
                        pr.head.ref_field.as_str(),
                        ARGS.local_repo.clone(),
                    )
                    .await
                    {
                        Ok(report) => {
                            // post report as github comment
                            match crab
                                .issues("AOSC-Dev", "aosc-os-abbs")
                                .create_comment(pr_number, report)
                                .await
                            {
                                Ok(comment) => {
                                    bot.send_message(
                                        msg.chat.id,
                                        truncate(&format!(
                                            "Report posted as comment: {}",
                                            comment.html_url
                                        )),
                                    )
                                    .await?;
                                }
                                Err(err) => {
                                    bot.send_message(
                                        msg.chat.id,
                                        truncate(&format!(
                                            "Failed to create github comments: {err:?}."
                                        )),
                                    )
                                    .await?;
                                }
                            }
                        }
                        Err(err) => {
                            bot.send_message(
                                msg.chat.id,
                                truncate(&format!("Failed to generate dickens report: {err:?}.")),
                            )
                            .await?;
                        }
                    },
                    Err(err) => {
                        bot.send_message(
                            msg.chat.id,
                            truncate(&format!("Failed to get pr info: {err:?}.")),
                        )
                        .await?;
                    }
                }
            }
            Err(err) => {
                bot.send_message(msg.chat.id, truncate(&format!("Bad PR number: {err:?}")))
                    .await?;
            }
        },
        Command::QA(arguments) => {
            let parts: Vec<&str> = arguments.split(' ').collect();
            if parts.len() == 2
                && ALL_ARCH.contains(&parts[0])
                && ["lagging", "missing"].contains(&parts[1])
            {
                let arch = parts[0];
                let ty = parts[1];
                let client = reqwest::Client::new();
                match client
                    .get(format!(
                        "https://aosc-packages.cth451.me/{}/{}/stable?type=json&page=all",
                        ty, arch
                    ))
                    .send()
                    .await
                {
                    Ok(resp) => match resp.json::<QAResponse>().await {
                        Ok(qa) => {
                            for pkg in qa.packages {
                                pipeline_new_and_report(
                                    &bot,
                                    pool.clone(),
                                    "stable",
                                    &pkg.name,
                                    arch,
                                    &msg,
                                )
                                .await?;
                            }
                        }
                        Err(err) => {
                            bot.send_message(
                                msg.chat.id,
                                truncate(&format!("Failed to parse http response: {err:?}",)),
                            )
                            .await?;
                        }
                    },
                    Err(err) => {
                        bot.send_message(
                            msg.chat.id,
                            truncate(&format!("Failed to get http response: {err:?}")),
                        )
                        .await?;
                    }
                }
                return Ok(());
            }

            bot.send_message(
                msg.chat.id,
                format!(
                    "Got invalid qa command: {arguments}. \n\n{}",
                    Command::descriptions()
                ),
            )
            .await?;
        }
        Command::Restart(arguments) => match str::parse::<i32>(&arguments) {
            Ok(job_id) => match job_restart(pool, job_id).await {
                Ok(new_job) => {
                    bot.send_message(
                        msg.chat.id,
                        truncate(&format!("Restarted as job #{}", new_job.id)),
                    )
                    .await?;
                }
                Err(err) => {
                    bot.send_message(
                        msg.chat.id,
                        truncate(&format!("Failed to restart job: {err:?}")),
                    )
                    .await?;
                }
            },
            Err(err) => {
                bot.send_message(msg.chat.id, truncate(&format!("Bad job ID: {err:?}")))
                    .await?;
            }
        },
        Command::Bump(package) => {
            let app_private_key = match ARGS.github_app_key.as_ref() {
                Some(p) => p,
                None => {
                    bot.send_message(msg.chat.id, "Got Error: GITHUB_APP_ID is not set")
                        .await?;
                    return Ok(());
                }
            };

            let secret = match ARGS.github_secret.as_ref() {
                Some(s) => s,
                None => {
                    bot.send_message(msg.chat.id, "GITHUB_SECRET is not set")
                        .await?;
                    return Ok(());
                }
            };

            let token = match get_github_token(&msg.chat.id, secret).await {
                Ok(s) => s.access_token,
                Err(e) => {
                    bot.send_message(msg.chat.id, truncate(&format!("Got error: {e:?}")))
                        .await?;
                    return Ok(());
                }
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

            let user = match get_user(pool.clone(), msg.chat.id, token.clone()).await {
                Ok(user) => user,
                Err(err) => {
                    bot.send_message(
                        msg.chat.id,
                        truncate(&format!("Failed to get user info: {:?}", err)),
                    )
                    .await?;
                    return Ok(());
                }
            };

            let mut coauthor_parts = vec![];
            if let Some(name) = &user.github_name {
                coauthor_parts.push(name.clone());
            }
            if let Some(login) = &user.github_login {
                coauthor_parts.push(format!("(@{})", login));
            }
            if let Some(email) = &user.github_email {
                coauthor_parts.push(format!("<{}>", email));
            }
            let coauthor = coauthor_parts.join(" ");

            match find_update_and_update_checksum(&package, &ARGS.abbs_path, &coauthor).await {
                Ok(f) => {
                    match buildit_utils::github::open_pr(
                        app_private_key,
                        &token,
                        id,
                        OpenPRRequest {
                            git_ref: f.branch,
                            abbs_path: ARGS.abbs_path.clone(),
                            packages: f.package,
                            title: f.title,
                            tags: None,
                            archs: None,
                        },
                    )
                    .await
                    {
                        Ok((pr_number, url)) => {
                            bot.send_message(
                                msg.chat.id,
                                truncate(&format!("Successfully opened PR: {url}")),
                            )
                            .await?;

                            create_pipeline_from_pr(pool.clone(), pr_number, None, &msg, &bot)
                                .await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id,
                                truncate(&format!("Failed to open PR: {:?}", e)),
                            )
                            .await?;
                        }
                    }
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        truncate(&format!("Failed to find update: {:?}", e)),
                    )
                    .await?;
                }
            };
        }
        Command::Roll => match roll().await {
            Ok(pkgs) => {
                let mut s = String::new();
                for i in pkgs {
                    s.push_str(&i.to_string());
                    s.push_str("\n");
                }

                bot.send_message(msg.chat.id, truncate(&s)).await?;
            }
            Err(e) => {
                bot.send_message(
                    msg.chat.id,
                    truncate(&format!("Failed to roll packages: {}", e)),
                )
                .await?;
            }
        },
    };

    Ok(())
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
struct UpdatePkg {
    name: String,
    before: String,
    after: String,
    warnings: Vec<String>,
}

impl Display for UpdatePkg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} -> {}", self.name, self.before, self.after)?;

        if !self.warnings.is_empty() {
            write!(f, " ({})", self.warnings.join("; "))?;
        }

        Ok(())
    }
}

async fn roll() -> anyhow::Result<Vec<UpdatePkg>> {
    let client = ClientBuilder::new().user_agent("buildit").build()?;
    let resp = client
        .get("https://github.com/AOSC-Dev/anicca/raw/main/pkgsupdate.json")
        .send()
        .await?;

    let resp = resp.error_for_status()?;
    let json = resp.json::<Vec<UpdatePkg>>().await?;

    let mut rng = thread_rng();
    let mut v = vec![];

    let mut count = 0;

    while count < 10 {
        let n = json.choose(&mut rng);
        if let Some(n) = n {
            if !v.contains(n) {
                v.push(n.clone());
                count += 1;
            }
        }
    }

    Ok(v)
}

fn truncate<'a>(text: &'a str) -> Cow<'a, str> {
    let text = if text.chars().count() > 1000 {
        console::truncate_str(text, 1000, "...")
    } else {
        Cow::Borrowed(text)
    };
    text
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
