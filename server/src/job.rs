use crate::{
    bot::http_rabbitmq_api,
    formatter::{to_html_build_result, to_markdown_build_result, FAILED, SUCCESS},
    github::{AMD64, ARM64, LOONGSON3, MIPS64R6EL, NOARCH, PPC64EL, RISCV64},
    ARGS,
};
use anyhow::anyhow;
use common::{ensure_job_queue, Job, JobError, JobOk, JobResult, JobSource};
use futures::StreamExt;
use lapin::{
    message::Delivery,
    options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions},
    types::FieldTable,
    BasicProperties, Channel, ConnectionProperties,
};
use log::{error, info, warn};
use octocrab::params::checks::CheckRunConclusion;
use octocrab::params::checks::CheckRunOutput;
use octocrab::{
    models::{CheckRunId, InstallationId},
    Octocrab,
};
use std::fmt::Write;
use std::time::Duration;
use teloxide::{prelude::*, types::ParseMode};

/// Observe job completion messages
pub async fn job_completion_worker_inner(bot: Bot, amqp_addr: &str) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    let _queue = channel
        .queue_declare(
            "job-completion",
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await?;

    let mut consumer = channel
        .basic_consume(
            "job-completion",
            "backend_server",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    let mut retry = None;

    while let Some(delivery) = consumer.next().await {
        let delivery = match delivery {
            Ok(delivery) => delivery,
            Err(err) => {
                error!("Got error in lapin delivery: {}", err);
                continue;
            }
        };

        if retry.map(|x| x < 5).unwrap_or(true) {
            match handle_success_message(&delivery, &bot, retry).await {
                HandleSuccessResult::Ok | HandleSuccessResult::DoNotRetry => {
                    ack_delivery(delivery).await
                }
                HandleSuccessResult::Retry(x) => {
                    retry = Some(x);
                    continue;
                }
            }
        } else {
            ack_delivery(delivery).await;
            retry = None;
        }
    }
    Ok(())
}

pub async fn ack_delivery(delivery: Delivery) {
    if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
        warn!(
            "Failed to delete job result {:?}, error: {:?}",
            delivery, err
        );
    } else {
        info!("Finished processing job result {:?}", delivery.delivery_tag);
    }
}

pub enum HandleSuccessResult {
    Ok,
    Retry(u8),
    DoNotRetry,
}

async fn handle_success_message(
    delivery: &Delivery,
    bot: &Bot,
    retry: Option<u8>,
) -> HandleSuccessResult {
    match serde_json::from_slice::<JobResult>(&delivery.data) {
        Ok(result) => {
            match result {
                JobResult::Ok(job) => {
                    info!("Processing job result {:?} ...", job);

                    let JobOk {
                        job: job_parent,
                        successful_packages,
                        pushpkg_success,
                        ..
                    } = &job;

                    let success = job_parent
                        .packages
                        .iter()
                        .all(|x| successful_packages.contains(x))
                        && *pushpkg_success;

                    if let JobSource::Telegram(id) = job_parent.source {
                        let s = to_html_build_result(&job, success);

                        if let Err(e) = bot
                            .send_message(ChatId(id), &s)
                            .parse_mode(ParseMode::Html)
                            .disable_web_page_preview(true)
                            .await
                        {
                            error!("{}", e);
                            return update_retry(retry);
                        }
                    }

                    // if associated with github pr, update comments
                    let new_content = to_markdown_build_result(&job, success);
                    if let Some(pr_num) = job_parent.github_pr {
                        let crab = match octocrab::Octocrab::builder()
                            .user_access_token(ARGS.github_access_token.clone())
                            .build()
                        {
                            Ok(crab) => crab,
                            Err(e) => {
                                error!("{e}");
                                return HandleSuccessResult::DoNotRetry;
                            }
                        };

                        let comments = crab
                            .issues("AOSC-Dev", "aosc-os-abbs")
                            .list_comments(pr_num)
                            .send()
                            .await;

                        let comments = match comments {
                            Ok(c) => c,
                            Err(e) => {
                                error!("{e}");
                                return update_retry(retry);
                            }
                        };

                        for c in comments {
                            if c.user.login == "aosc-buildit-bot" {
                                let body = c.body.unwrap_or_else(String::new);
                                if !body
                                    .split_ascii_whitespace()
                                    .next()
                                    .map(|x| x == SUCCESS || x == FAILED)
                                    .unwrap_or(false)
                                {
                                    continue;
                                }

                                for line in body.split('\n') {
                                    let arch = line.strip_prefix("Architecture:").map(|x| x.trim());
                                    if arch.map(|x| x == job_parent.arch).unwrap_or(false) {
                                        if let Err(e) = crab
                                            .issues("AOSC-Dev", "aosc-os-abbs")
                                            .delete_comment(c.id)
                                            .await
                                        {
                                            error!("{e}");
                                            return update_retry(retry);
                                        }
                                    }
                                }
                            }
                        }

                        // Disable comment posting, since we have check run reporting
                        /*
                        if let Err(e) = crab
                            .issues("AOSC-Dev", "aosc-os-abbs")
                            .create_comment(pr_num, new_content.clone())
                            .await
                        {
                            error!("{e}");
                            return update_retry(retry);
                        }
                        */

                        // update checklist
                        let pr = match crab.pulls("AOSC-Dev", "aosc-os-abbs").get(pr_num).await {
                            Ok(pr) => pr,
                            Err(e) => {
                                error!("{e}");
                                return update_retry(retry);
                            }
                        };

                        let body = if let Some(body) = pr.body {
                            body
                        } else {
                            return HandleSuccessResult::DoNotRetry;
                        };

                        let pr_arch = match job_parent.arch.as_str() {
                            "amd64" if job_parent.noarch => NOARCH,
                            "amd64" => AMD64,
                            "arm64" => ARM64,
                            "loongson3" => LOONGSON3,
                            "mips64r6el" => MIPS64R6EL,
                            "ppc64el" => PPC64EL,
                            "riscv64" => RISCV64,
                            "loongarch64" => {
                                // FIXME: loongarch64 does not in mainline for now
                                return HandleSuccessResult::Ok;
                            }
                            x => {
                                error!("Unknown architecture: {x}");
                                return HandleSuccessResult::DoNotRetry;
                            }
                        };

                        let body = if success {
                            body.replace(&format!("- [ ] {pr_arch}"), &format!("- [x] {pr_arch}"));
                        } else {
                            body.replace(&format!("- [x] {pr_arch}"), &format!("- [ ] {pr_arch}"));
                        };

                        if let Err(e) = crab
                            .pulls("AOSC-Dev", "aosc-os-abbs")
                            .update(pr_num)
                            .body(body)
                            .send()
                            .await
                        {
                            error!("{e}");
                            return update_retry(retry);
                        }
                    }

                    // if associated with github check run, update status
                    if let Some(github_check_run_id) = job_parent.github_check_run_id {
                        // authenticate with github app
                        match get_crab_github_installation().await {
                            Ok(Some(crab)) => {
                                let handler = crab.checks("AOSC-Dev", "aosc-os-abbs");
                                let output = CheckRunOutput {
                                    title: format!(
                                        "Built {} packages in {:?}",
                                        job.successful_packages.len(),
                                        job.elapsed
                                    ),
                                    summary: new_content,
                                    text: None,
                                    annotations: vec![],
                                    images: vec![],
                                };
                                let mut builder = handler
                                    .update_check_run(CheckRunId(github_check_run_id))
                                    .status(octocrab::params::checks::CheckRunStatus::Completed)
                                    .output(output)
                                    .conclusion(if success {
                                        CheckRunConclusion::Success
                                    } else {
                                        CheckRunConclusion::Failure
                                    });

                                if let Some(log) = job.log {
                                    builder = builder.details_url(log);
                                }

                                if let Err(e) = builder.send().await {
                                    error!("{e}");
                                    return update_retry(retry);
                                }
                            }
                            Ok(None) => {
                                // github app unavailable
                            }
                            Err(err) => {
                                warn!("Failed to get installation token: {}", err);
                                return update_retry(retry);
                            }
                        }
                    }
                }
                JobResult::Error(job) => {
                    let JobError {
                        job: job_parent,
                        worker,
                        error,
                    } = job;

                    match job_parent.source {
                        JobSource::Telegram(id) => {
                            if let Err(e) = bot
                                .send_message(
                                    ChatId(id),
                                    format!(
                                        "{}({}) build packages: {:?} Got Error: {}",
                                        worker.hostname,
                                        job_parent.arch,
                                        job_parent.packages,
                                        error
                                    ),
                                )
                                .await
                            {
                                error!("{e}");
                                return update_retry(retry);
                            }
                        }
                        JobSource::Github(num) => {
                            let crab = match octocrab::Octocrab::builder()
                                .user_access_token(ARGS.github_access_token.clone())
                                .build()
                            {
                                Ok(crab) => crab,
                                Err(e) => {
                                    error!("{e}");
                                    return HandleSuccessResult::DoNotRetry;
                                }
                            };

                            if let Err(e) = crab
                                .issues("AOSC-Dev", "aosc-os-abbs")
                                .create_comment(
                                    num,
                                    format!(
                                        "{}({}) build packages: {:?} Got Error: {}",
                                        worker.hostname,
                                        job_parent.arch,
                                        job_parent.packages,
                                        error
                                    ),
                                )
                                .await
                            {
                                error!("{e}");
                                return update_retry(retry);
                            }
                        }
                    }
                }
            }
        }
        Err(err) => {
            warn!("Got invalid json in job-completion: {}", err);
        }
    }

    HandleSuccessResult::Ok
}

pub async fn get_ready_message(
    amqp_addr: &str,
    archs: &[&str],
) -> anyhow::Result<Vec<(String, String)>> {
    let mut res = vec![];
    let conn = lapin::Connection::connect(amqp_addr, ConnectionProperties::default()).await?;
    let channel = conn.create_channel().await?;

    for i in archs {
        ensure_job_queue(&format!("job-{i}"), &channel).await?;
        let api = ARGS
            .rabbitmq_queue_api
            .as_ref()
            .ok_or_else(|| anyhow!("rabbitmq_queue_api is not set"))?;

        let api_root = http_rabbitmq_api(api, format!("job-{i}")).await?;
        let ready = api_root
            .get("messages_ready")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| anyhow!("Failed to get ready message count"))?;

        if ready > 0 {
            let client = reqwest::Client::new();
            let resp: Vec<serde_json::Value> = client
                .post(format!("{api}job-{i}/get"))
                .header("Content-type", "application/json")
                .json(&serde_json::json!({
                    "count": ready,
                    "requeue": "true",
                    "encoding": "auto",
                    "truncate": "50000",
                    "ackmode": "ack_requeue_true",
                }))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            let mut msg = String::new();
            // parse payload as Job
            for entry in resp {
                if let Some(job) = entry
                    .as_object()
                    .and_then(|e| e.get("payload"))
                    .and_then(|v| v.as_str())
                {
                    writeln!(&mut msg, "{}", job)?;
                }
            }

            res.push((i.to_string(), msg));
        }
    }

    Ok(res)
}

pub fn update_retry(retry: Option<u8>) -> HandleSuccessResult {
    match retry {
        Some(retry) => HandleSuccessResult::Retry(retry + 1),
        None => HandleSuccessResult::Retry(1),
    }
}

pub async fn job_completion_worker(bot: Bot, amqp_addr: String) -> anyhow::Result<()> {
    loop {
        info!("Starting job completion worker ...");
        if let Err(err) = job_completion_worker_inner(bot.clone(), &amqp_addr).await {
            error!("Got error while starting job completion worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Create octocrab instance authenticated as github installation
async fn get_crab_github_installation() -> anyhow::Result<Option<Octocrab>> {
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

pub async fn send_build_request(
    git_ref: &str,
    packages: &[String],
    archs: &[&str],
    github_pr: Option<u64>,
    source: JobSource,
    sha: &str,
    channel: &Channel,
) -> anyhow::Result<()> {
    // authenticate with github app
    let crab = match get_crab_github_installation().await {
        Ok(Some(crab)) => Some(crab),
        Ok(None) => {
            // github app unavailable
            None
        }
        Err(err) => {
            warn!("Failed to build octocrab: {}", err);
            None
        }
    };

    // for each arch, create a job
    for arch in archs {
        // create github check run
        let mut github_check_run_id = None;
        if let Some(crab) = &crab {
            match crab
                .checks("AOSC-Dev", "aosc-os-abbs")
                .create_check_run(format!("buildit {}", arch), sha)
                .status(octocrab::params::checks::CheckRunStatus::InProgress)
                .send()
                .await
            {
                Ok(check_run) => {
                    github_check_run_id = Some(check_run.id.0);
                }
                Err(err) => {
                    warn!("Failed to create check run: {}", err);
                }
            }
        }

        let job = Job {
            packages: packages.iter().map(|s| s.to_string()).collect(),
            git_ref: git_ref.to_string(),
            arch: if arch == &"noarch" {
                "amd64".to_string()
            } else {
                arch.to_string()
            },
            source: source.clone(),
            github_pr,
            noarch: arch == &"noarch",
            sha: sha.to_string(),
            enqueue_time: Some(chrono::Utc::now()),
            github_check_run_id,
        };

        info!("Adding job to message queue {:?} ...", job);

        // each arch has its own queue
        let queue_name = format!("job-{}", job.arch);
        ensure_job_queue(&queue_name, channel).await?;

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
