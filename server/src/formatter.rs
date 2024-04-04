use std::borrow::Cow;

use common::JobOk;

use crate::models::{Job, Pipeline};

pub const SUCCESS: &str = "✅️";
pub const FAILED: &str = "❌";

pub fn to_html_new_pipeline_summary(
    pipeline_id: i32,
    git_ref: &str,
    github_pr: Option<u64>,
    archs: &[&str],
    packages: &[&str],
) -> String {
    format!(
        r#"<b><u>New Pipeline Summary</u></b>

<b>Pipeline</b>: <a href="https://buildit.aosc.io/pipelines/{}">#{}</a>
<b>Git reference</b>: {}{}
<b>Architecture(s)</b>: {}
<b>Package(s)</b>: {}"#,
        pipeline_id,
        pipeline_id,
        git_ref,
        if let Some(pr) = github_pr {
            format!("\n<b>GitHub PR</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/pull/{}\">#{}</a>", pr, pr)
        } else {
            String::new()
        },
        archs.join(", "),
        packages.join(", "),
    )
}

pub fn to_html_build_result(
    pipeline: &Pipeline,
    job: &Job,
    job_ok: &JobOk,
    worker_hostname: &str,
    worker_arch: &str,
    success: bool,
) -> String {
    let JobOk {
        successful_packages,
        failed_package,
        skipped_packages,
        log_url,
        elapsed_secs,
        ..
    } = job_ok;

    format!(
        r#"{} Job completed on {} ({})

<b>Job</b>: {}
<b>Pipeline</b>: {}
<b>Enqueue time</b>: {}
<b>Time elapsed</b>: {}
<b>Git commit</b>: {}
<b>Git branch</b>: {}
{}<b>Architecture</b>: {}
<b>Package(s) to build</b>: {}
<b>Package(s) successfully built</b>: {}
<b>Package(s) failed to build</b>: {}
<b>Package(s) not built due to previous build failure</b>: {}

{}"#,
        if success { SUCCESS } else { FAILED },
        worker_hostname,
        worker_arch,
        format!(
            "<a href=\"https://buildit.aosc.io/jobs/{}\">#{}</a>",
            job.id, job.id
        ),
        format!(
            "<a href=\"https://buildit.aosc.io/pipelines/{}\">#{}</a>",
            pipeline.id, pipeline.id
        ),
        format!("{}", job.creation_time),
        format!("{}s", elapsed_secs),
        format!(
            "<a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/commit/{}\">{}</a>",
            pipeline.git_sha,
            &pipeline.git_sha[..8]
        ),
        format!(
            "<a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/tree/{}\">{}</a>",
            pipeline.git_branch, &pipeline.git_branch
        ),
        if let Some(pr) = pipeline.github_pr {
            format!(
                "<b>GitHub PR</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/pull/{}\">#{}</a>\n",
                pr, pr
            )
        } else {
            String::new()
        },
        job.arch,
        job.packages.replace(",", ", "),
        &successful_packages.join(", "),
        &failed_package.clone().unwrap_or(String::from("None")),
        &skipped_packages.join(", "),
        if let Some(log) = log_url {
            Cow::Owned(format!("<a href=\"{}\">Build Log >></a>", log))
        } else {
            Cow::Borrowed("Failed to push log! See <code>/buildroots/buildit/buildit/push_failed_logs</code> to see log.")
        }
    )
}

pub fn to_markdown_build_result(
    pipeline: &Pipeline,
    job: &Job,
    job_ok: &JobOk,
    worker_hostname: &str,
    worker_arch: &str,
    success: bool,
) -> String {
    let JobOk {
        successful_packages,
        failed_package,
        skipped_packages,
        log_url,
        elapsed_secs,
        ..
    } = job_ok;

    format!(
        "{} Job completed on {} \\({}\\)\n\n**Job**: {}\n**Pipeline**: {}\n**Enqueue time**: {}\n**Time elapsed**: {}s\n{}{}**Architecture**: {}\n**Package\\(s\\) to build**: {}\n**Package\\(s\\) successfully built**: {}\n**Package\\(s\\) failed to build**: {}\n**Package\\(s\\) not built due to previous build failure**: {}\n\n{}\n",
        if success { SUCCESS } else { FAILED },
        worker_hostname,
        worker_arch,
        format!("[#{}](https://buildit.aosc.io/jobs/{})", job.id, job.id),
        format!("[#{}](https://buildit.aosc.io/pipelines/{})", pipeline.id, pipeline.id),
        teloxide::utils::markdown::escape(&job.creation_time.to_string()),
        elapsed_secs,
        format!("**Git commit**: [{}](https://github.com/AOSC-Dev/aosc-os-abbs/commit/{})\n", &pipeline.git_sha[..8], pipeline.git_sha),
        format!("**Git branch**: [{}](https://github.com/AOSC-Dev/aosc-os-abbs/tree/{})\n", &pipeline.git_branch, pipeline.git_branch),
        job.arch,
        teloxide::utils::markdown::escape(&job.packages.replace(",", ", ")),
        teloxide::utils::markdown::escape(&successful_packages.join(", ")),
        teloxide::utils::markdown::escape(&failed_package.clone().unwrap_or(String::from("None"))),
        teloxide::utils::markdown::escape(&skipped_packages.join(", ")),
        if let Some(log) = log_url {
            Cow::Owned(format!("[Build Log \\>\\>]({})", log))
        } else {
            Cow::Borrowed("Failed to push log! See `/buildroots/buildit/buildit/push_failed_logs` to see log.")
        }
    )
}

pub fn code_repr_string(s: &str) -> String {
    format!("<code>{s}</code>")
}

#[test]
fn test_format_html_new_pipeline_summary() {
    let s = to_html_new_pipeline_summary(1, "fd-9.0.0", Some(4992), &["amd64"], &["fd"]);
    assert_eq!(s, "<b><u>New Pipeline Summary</u></b>\n\n<b>Pipeline</b>: <a href=\"https://buildit.aosc.io/pipelines/1\">#1</a>\n<b>Git reference</b>: fd-9.0.0\n<b>GitHub PR</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/pull/4992\">#4992</a>\n<b>Architecture(s)</b>: amd64\n<b>Package(s)</b>: fd")
}

#[test]
fn test_format_html_build_result() {
    use chrono::DateTime;
    use common::JobOk;

    let pipeline = Pipeline {
        id: 1,
        packages: "fd".to_string(),
        archs: "amd64".to_string(),
        git_branch: "fd-9.0.0".to_string(),
        git_sha: "34acef168fc5ec454d3825fc864964951b130b49".to_string(),
        creation_time: DateTime::from_timestamp(61, 0).unwrap(),
        source: "telegram".to_string(),
        github_pr: Some(4992),
        telegram_user: None,
        creator_user_id: None,
    };

    let job = Job {
        id: 1,
        pipeline_id: 1,
        packages: "fd,fd2".to_string(),
        arch: "amd64".to_string(),
        creation_time: DateTime::from_timestamp(61, 0).unwrap(),
        status: "success".to_string(),
        github_check_run_id: None,
        build_success: Some(true),
        pushpkg_success: Some(true),
        successful_packages: Some("fd".to_string()),
        failed_package: None,
        skipped_packages: Some("".to_string()),
        log_url: Some("https://pastebin.aosc.io/paste/c0rWzj4EsSC~CVXs2qXtFw".to_string()),
        finish_time: Some(DateTime::from_timestamp(61, 0).unwrap()),
        assign_time: Some(DateTime::from_timestamp(61, 0).unwrap()),
        error_message: None,
        elapsed_secs: Some(888),
        assigned_worker_id: Some(1),
        built_by_worker_id: Some(1),
        require_min_core: None,
        require_min_disk: None,
        require_min_total_mem: None,
        require_min_total_mem_per_core: None,
    };

    let job_ok = JobOk {
        build_success: true,
        successful_packages: vec!["fd".to_string()],
        failed_package: None,
        skipped_packages: vec![],
        log_url: Some("https://pastebin.aosc.io/paste/c0rWzj4EsSC~CVXs2qXtFw".to_string()),
        elapsed_secs: 888,
        pushpkg_success: true,
    };

    let worker_hostname = "Yerus";
    let worker_arch = "amd64";

    let s = to_html_build_result(&pipeline, &job, &job_ok, worker_hostname, worker_arch, true);

    assert_eq!(s, "✅\u{fe0f} Job completed on Yerus (amd64)\n\n<b>Job</b>: <a href=\"https://buildit.aosc.io/jobs/1\">#1</a>\n<b>Pipeline</b>: <a href=\"https://buildit.aosc.io/pipelines/1\">#1</a>\n<b>Enqueue time</b>: 1970-01-01 00:01:01 UTC\n<b>Time elapsed</b>: 888s\n<b>Git commit</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/commit/34acef168fc5ec454d3825fc864964951b130b49\">34acef16</a>\n<b>Git branch</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/tree/fd-9.0.0\">fd-9.0.0</a>\n<b>GitHub PR</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/pull/4992\">#4992</a>\n<b>Architecture</b>: amd64\n<b>Package(s) to build</b>: fd, fd2\n<b>Package(s) successfully built</b>: fd\n<b>Package(s) failed to build</b>: None\n<b>Package(s) not built due to previous build failure</b>: \n\n<a href=\"https://pastebin.aosc.io/paste/c0rWzj4EsSC~CVXs2qXtFw\">Build Log >></a>")
}
