use crate::{get_memory_bytes, Args};
use chrono::Local;
use common::{JobOk, WorkerJobUpdateRequest, WorkerPollRequest, WorkerPollResponse};
use flume::Sender;
use futures_util::future::try_join3;
use log::{error, info, warn};
use std::{
    path::Path,
    process::{Output, Stdio},
    time::{Duration, Instant},
};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    time::sleep,
};
use tokio_tungstenite::tungstenite::Message;

async fn get_output_logged(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    logs: &mut Vec<u8>,
    tx: Sender<Message>,
) -> anyhow::Result<Output> {
    let begin = Instant::now();
    let msg = format!(
        "{}: Running `{} {}` in `{}`\n",
        Local::now(),
        cmd,
        args.join(" "),
        cwd.display()
    );
    logs.extend(msg.as_bytes());
    info!("{}", msg.trim());

    let mut output = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // learn from tokio wait_with_output
    async fn read_and_send<A: AsyncRead + Unpin>(
        io: &mut Option<A>,
        tx: Sender<Message>,
    ) -> tokio::io::Result<String> {
        let mut res = String::new();
        if let Some(io) = io.as_mut() {
            // do not use next_line: stream may contain invalid utf-8!
            let mut reader = BufReader::new(io);
            let mut buffer = vec![];
            loop {
                match reader.read_until(b'\n', &mut buffer).await {
                    // EOF
                    Ok(0) => break,
                    Ok(_size) => {
                        // drop trailing \n or \r\n
                        if buffer.ends_with(&[b'\n']) {
                            buffer.pop();
                            if buffer.ends_with(&[b'\r']) {
                                buffer.pop();
                            }
                        }

                        // convert \r to \n
                        for line in String::from_utf8_lossy(&buffer).split("\r") {
                            tx.send_async(Message::Text(line.to_string())).await.ok();
                            res += &line;
                            res += "\n";
                        }
                    }
                    Err(err) => {
                        warn!("Failed to read next line: {err}");
                        break;
                    }
                }
                buffer.clear();
            }
        }
        Ok(res)
    }

    let mut stdout_pipe = output.stdout.take();
    let stdout_future = read_and_send(&mut stdout_pipe, tx.clone());
    let mut stderr_pipe = output.stderr.take();
    let stderr_future = read_and_send(&mut stderr_pipe, tx.clone());

    let (status, stdout, stderr) = try_join3(output.wait(), stdout_future, stderr_future).await?;

    let elapsed = begin.elapsed();

    logs.extend(
        format!(
            "{}: `{} {}` finished in {:?} with {}\n",
            Local::now(),
            cmd,
            args.join(" "),
            elapsed,
            status.success()
        )
        .as_bytes(),
    );
    logs.extend("STDOUT:\n".as_bytes());
    logs.extend(stdout.as_bytes());
    logs.extend("STDERR:\n".as_bytes());
    logs.extend(stderr.as_bytes());

    Ok(Output {
        status,
        stdout: stdout.into_bytes(),
        stderr: stderr.into_bytes(),
    })
}

/// Run command and retry until it succeeds
async fn run_logged_with_retry(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    logs: &mut Vec<u8>,
    tx: Sender<Message>,
) -> anyhow::Result<bool> {
    for i in 0..5 {
        if i > 0 {
            info!("Attempt #{i} to run `{cmd} {}`", args.join(" "));
        }
        match get_output_logged(cmd, args, cwd, logs, tx.clone()).await {
            Ok(output) => {
                if output.status.success() {
                    return Ok(true);
                } else {
                    warn!(
                        "Running `{cmd} {}` exited with {}",
                        args.join(" "),
                        output.status
                    );
                }
            }
            Err(err) => {
                warn!("Running `{cmd} {}` failed with {err}", args.join(" "));
            }
        }
        // exponential backoff
        sleep(Duration::from_secs(1 << i)).await;
    }
    warn!("Failed too many times running `{cmd} {}`", args.join(" "));
    Ok(false)
}

async fn build(
    job: &WorkerPollResponse,
    tree_path: &Path,
    args: &Args,
    tx: Sender<Message>,
) -> anyhow::Result<WorkerJobUpdateRequest> {
    let begin = Instant::now();
    let mut successful_packages = vec![];
    let mut failed_package = None;
    let mut skipped_packages = vec![];
    let mut build_success = false;
    let mut logs = vec![];

    let mut output_path = args.ciel_path.clone();
    output_path.push(format!("OUTPUT-{}", job.git_branch));

    // clear output directory
    if output_path.exists() {
        get_output_logged("rm", &["-rf", "debs"], &output_path, &mut logs, tx.clone()).await?;
    }

    // switch to git ref
    let git_fetch_succeess = run_logged_with_retry(
        "git",
        &[
            "fetch",
            "https://github.com/AOSC-Dev/aosc-os-abbs.git",
            &job.git_branch,
        ],
        tree_path,
        &mut logs,
        tx.clone(),
    )
    .await?;

    let mut pushpkg_success = false;

    if git_fetch_succeess {
        // try to switch branch, but allow it to fail:
        // ensure branch exists
        get_output_logged(
            "git",
            &["checkout", "-b", &job.git_branch],
            tree_path,
            &mut logs,
            tx.clone(),
        )
        .await?;
        // checkout to branch
        get_output_logged(
            "git",
            &["checkout", &job.git_branch],
            tree_path,
            &mut logs,
            tx.clone(),
        )
        .await?;

        // switch to the commit by sha
        // to avoid race condition, resolve branch name to sha in server
        let output = get_output_logged(
            "git",
            &["reset", &job.git_sha, "--hard"],
            tree_path,
            &mut logs,
            tx.clone(),
        )
        .await?;

        if output.status.success() {
            // update container
            get_output_logged(
                "ciel",
                &["update-os"],
                &args.ciel_path,
                &mut logs,
                tx.clone(),
            )
            .await?;

            // build packages
            let mut ciel_args = vec!["build", "-i", &args.ciel_instance];
            ciel_args.extend(job.packages.split(','));
            let output =
                get_output_logged("ciel", &ciel_args, &args.ciel_path, &mut logs, tx.clone())
                    .await?;

            build_success = output.status.success();

            // parse output
            // match acbs/acbs/util.py
            let mut found_banner = false;
            let mut found_acbs_build = false;
            let mut found_failed_package = false;
            let mut found_packages_built = false;
            let mut found_packages_not_built = false;

            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if line.contains("========================================") {
                    found_banner = true;
                } else if line.contains("ACBS Build") {
                    found_acbs_build = true;
                } else if found_banner && found_acbs_build {
                    if line.starts_with("Failed package:") {
                        found_failed_package = true;
                        found_packages_built = false;
                        found_packages_not_built = false;
                    } else if line.starts_with("Package(s) built:") {
                        found_failed_package = false;
                        found_packages_built = true;
                        found_packages_not_built = false;
                    } else if line
                        .starts_with("Package(s) not built due to previous build failure:")
                    {
                        found_failed_package = false;
                        found_packages_built = false;
                        found_packages_not_built = true;
                    } else if line.contains('(') {
                        // e.g. bash (amd64 @ 5.2.15-0)
                        if let Some(package_name) = line.split(' ').next() {
                            if found_packages_built {
                                successful_packages.push(package_name.to_string());
                            } else if found_failed_package {
                                failed_package = Some(package_name.to_string());
                            } else if found_packages_not_built {
                                skipped_packages.push(package_name.to_string());
                            }
                        }
                    } else if line.is_empty() {
                        found_failed_package = false;
                        found_packages_built = false;
                        found_packages_not_built = false;
                    }
                }
            }

            if build_success {
                if let Some(upload_ssh_key) = &args.upload_ssh_key {
                    let mut pushpkg_args = vec![
                        "--host",
                        &args.rsync_host,
                        "-i",
                        upload_ssh_key,
                        "maintainers",
                        &job.git_branch,
                    ];
                    if &args.pushpkg_options != "" {
                        pushpkg_args.insert(0, &args.pushpkg_options);
                    }
                    if &job.git_branch != "stable" {
                        // allow force push if noarch and non stable
                        pushpkg_args.insert(0, "--force-push-noarch-package");
                    }
                    pushpkg_success = run_logged_with_retry(
                        "pushpkg",
                        &pushpkg_args,
                        &output_path,
                        &mut logs,
                        tx.clone(),
                    )
                    .await?;
                }
            }
        }
    }

    let file_name = format!(
        "{}-{}-{}-{}-{}.txt",
        job.job_id,
        job.git_branch,
        args.arch,
        gethostname::gethostname().to_string_lossy(),
        Local::now().format("%Y-%m-%d-%H:%M:%S")
    );

    let path = format!("/tmp/{file_name}");
    fs::write(&path, logs).await?;

    let mut log_url = None;
    if let Some(upload_ssh_key) = &args.upload_ssh_key {
        let mut scp_log = vec![];
        if run_logged_with_retry(
            "scp",
            &[
                "-i",
                &upload_ssh_key,
                &path,
                &format!("maintainers@{}:/buildit/logs", args.rsync_host),
            ],
            &tree_path,
            &mut scp_log,
            tx,
        )
        .await?
        {
            fs::remove_file(&path).await?;
            log_url = Some(format!("https://buildit.aosc.io/logs/{file_name}"));
        } else {
            error!(
                "Failed to scp log to repo: {}",
                String::from_utf8_lossy(&scp_log)
            );
        };
    }

    if log_url.is_none() {
        let dir = Path::new("./push_failed_logs");
        let to = dir.join(file_name);
        fs::create_dir_all(dir).await?;
        fs::copy(&path, to).await?;
    }

    let result = WorkerJobUpdateRequest {
        hostname: gethostname::gethostname().to_string_lossy().to_string(),
        arch: args.arch.clone(),
        worker_secret: args.worker_secret.clone(),
        job_id: job.job_id,
        result: common::JobResult::Ok(JobOk {
            build_success: build_success,
            successful_packages,
            failed_package,
            skipped_packages,
            log_url,
            elapsed_secs: begin.elapsed().as_secs() as i64,
            pushpkg_success,
        }),
    };

    Ok(result)
}

async fn build_worker_inner(args: &Args, tx: Sender<Message>) -> anyhow::Result<()> {
    let mut tree_path = args.ciel_path.clone();
    tree_path.push("TREE");

    info!("Receiving new messages");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let req = WorkerPollRequest {
        hostname: hostname.clone(),
        arch: args.arch.clone(),
        worker_secret: args.worker_secret.clone(),
        memory_bytes: get_memory_bytes(),
        disk_free_space_bytes: fs2::free_space(std::env::current_dir()?)? as i64,
        logical_cores: num_cpus::get() as i32,
    };

    loop {
        if let Some(job) = client
            .post(format!("{}/api/worker/poll", args.server))
            .json(&req)
            .send()
            .await?
            .json::<Option<WorkerPollResponse>>()
            .await?
        {
            info!("Processing job {:?}", job);

            match build(&job, &tree_path, args, tx.clone()).await {
                Ok(result) => {
                    // post result
                    info!("Finished to run job {:?} with result {:?}", job, result);
                    client
                        .post(format!("{}/api/worker/job_update", args.server))
                        .json(&result)
                        .send()
                        .await?;
                }
                Err(err) => {
                    warn!("Failed to run job {:?} with err {:?}", job, err);
                    client
                        .post(format!("{}/api/worker/job_update", args.server))
                        .json(&WorkerJobUpdateRequest {
                            hostname: gethostname::gethostname().to_string_lossy().to_string(),
                            arch: args.arch.clone(),
                            worker_secret: args.worker_secret.clone(),
                            job_id: job.job_id,
                            result: common::JobResult::Error(err.to_string()),
                        })
                        .send()
                        .await?;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

pub async fn build_worker(args: Args, tx: Sender<Message>) -> ! {
    loop {
        info!("Starting build worker");
        if let Err(err) = build_worker_inner(&args, tx.clone()).await {
            warn!("Got error running heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
