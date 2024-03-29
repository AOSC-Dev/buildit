use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct WebhookComment {
    comment: Comment,
}

#[derive(Debug, Deserialize)]
struct Comment {
    issue_url: String,
    user: User,
    body: String,
}

#[derive(Debug, Deserialize)]
struct User {
    login: String,
}

/*
pub async fn get_webhooks_message(pool: deadpool_lapin::Pool) {
    info!("Starting github webhook worker");
    loop {
        if let Err(e) = get_webhooks_message_inner(pool.clone()).await {
            error!("Error getting webhooks message: {e}");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn get_webhooks_message_inner(pool: deadpool_lapin::Pool) -> anyhow::Result<()> {
    let conn = pool.get().await?;
    let channel = conn.create_channel().await?;
    let _queue = channel
        .queue_declare(
            "github-webhooks",
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await?;

    let mut consumer = channel
        .basic_consume(
            "github-webhooks",
            "",
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

        match serde_json::from_slice::<WebhookComment>(&delivery.data) {
            Ok(comment) => {
                match handle_webhook_comment(&comment, &ARGS.abbs_path, retry, &channel).await {
                    HandleSuccessResult::Ok | HandleSuccessResult::DoNotRetry => {
                        ack_delivery(delivery).await
                    }
                    HandleSuccessResult::Retry(r) => {
                        if r == 5 {
                            ack_delivery(delivery).await;
                            retry = None;
                            continue;
                        }

                        retry = Some(r);
                    }
                }
            }
            Err(e) => {
                error!("{e}");
                ack_delivery(delivery).await
            }
        }
    }

    Ok(())
}

async fn handle_webhook_comment(
    comment: &WebhookComment,
    path: &Path,
    retry: Option<u8>,
    channel: &Channel,
) -> HandleSuccessResult {
    info!("Got comment in lapin delivery: {:?}", comment);
    if !comment.comment.body.starts_with("@aosc-buildit-bot") {
        return HandleSuccessResult::DoNotRetry;
    }

    let body = comment
        .comment
        .body
        .trim()
        .split_ascii_whitespace()
        .skip(1)
        .collect::<Vec<_>>();

    info!("{body:?}");

    if body.first().map(|x| x != &"build").unwrap_or(true) {
        return HandleSuccessResult::DoNotRetry;
    }

    let num = match comment
        .comment
        .issue_url
        .split('/')
        .last()
        .and_then(|x| x.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Failed to get pr number"))
    {
        Ok(num) => num,
        Err(e) => {
            error!("{e}");
            return update_retry(retry);
        }
    };

    let pr = match octocrab::instance()
        .pulls("AOSC-Dev", "aosc-os-abbs")
        .get(num)
        .await
    {
        Ok(pr) => pr,
        Err(e) => {
            error!("{e}");
            return update_retry(retry);
        }
    };

    let packages = get_packages_from_pr(&pr);

    let archs = if let Some(archs) = body.get(1) {
        archs.split(',').collect::<Vec<_>>()
    } else {
        get_archs(path, &packages)
    };

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

    let is_org_user = is_org_user(&comment.comment.user.login).await;

    match is_org_user {
        Ok(true) => (),
        Ok(false) => {
            error!("{} is not a org user", comment.comment.user.login);
            return HandleSuccessResult::DoNotRetry;
        }
        Err(e) => {
            error!("{e}");
            return update_retry(retry);
        }
    }

    let crab = match octocrab::Octocrab::builder()
        .user_access_token(ARGS.github_access_token.clone())
        .build()
    {
        Ok(v) => v,
        Err(e) => {
            error!("{e}");
            return HandleSuccessResult::DoNotRetry;
        }
    };

    let path = &ARGS.abbs_path;

    if let Err(e) = update_abbs(branch, path).await {
        create_github_comment(&crab, retry, num, &e.to_string()).await;
    }

    let s = to_html_new_pipeline_summary(
        branch,
        Some(num),
        &archs,
        &packages.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
    );

    match send_build_request(
        branch,
        &packages,
        &archs,
        Some(num),
        JobSource::Github(num),
        sha,
        channel,
    )
    .await
    {
        Ok(()) => {
            let comments = crab
                .issues("AOSC-Dev", "aosc-os-abbs")
                .list_comments(num)
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

            create_github_comment(&crab, retry, num, &s).await
        }
        Err(e) => {
            error!("{e}");
            update_retry(retry)
        }
    }
}

async fn create_github_comment(
    crab: &Octocrab,
    retry: Option<u8>,
    num: u64,
    s: &str,
) -> HandleSuccessResult {
    if let Err(e) = crab
        .issues("AOSC-Dev", "aosc-os-abbs")
        .create_comment(num, s)
        .await
    {
        error!("{e}");
        return update_retry(retry);
    }

    HandleSuccessResult::Ok
}

async fn is_org_user(user: &str) -> anyhow::Result<bool> {
    let client = reqwest::Client::builder().user_agent("buildit").build()?;

    let resp = client
        .get(format!(
            "https://api.github.com/orgs/aosc-dev/public_members/{}",
            user
        ))
        .send()
        .await
        .and_then(|x| x.error_for_status());

    match resp {
        Ok(_) => Ok(true),
        Err(e) => match e.status() {
            Some(StatusCode::NOT_FOUND) => Ok(false),
            _ => bail!("Network is not reachable: {e}"),
        },
    }
}

*/
