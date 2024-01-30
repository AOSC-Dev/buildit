use std::borrow::Cow;

use common::JobOk;

pub const SUCCESS: &str = "✅️";
pub const FAILED: &str = "❌";

pub fn to_html_new_job_summary(
    git_ref: &str,
    github_pr: Option<u64>,
    archs: &[&str],
    packages: &[String],
) -> String {
    format!(
        r#"<b><u>New Job Summary</u></b>

<b>Git reference</b>: {}{}
<b>Architecture(s)</b>: {}
<b>Package(s)</b>: {}"#,
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

pub fn to_html_build_result(job: &JobOk, success: bool) -> String {
    let JobOk {
        job,
        successful_packages,
        failed_package,
        skipped_packages,
        log,
        worker,
        elapsed,
        ..
    } = job;

    format!(
        r#"{} Job completed on {} ({})

{}<b>Time elapsed</b>: {}
{}{}<b>Architecture</b>: {}
<b>Package(s) to build</b>: {}
<b>Package(s) successfully built</b>: {}
<b>Package(s) failed to build</b>: {}
<b>Package(s) not built due to previous build failure</b>: {}

{}"#,
        if success { SUCCESS } else { FAILED },
        &worker.hostname,
        worker.arch,
        if let Some(enqueue_time) = &job.enqueue_time {
            format!("<b>Enqueue time</b>: {}\n", enqueue_time)
        } else {
            String::new()
        },
        &format!("{:.2?}", elapsed),
        format!("<b>Git commit</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/commit/{}\">{}</a>\n", job.sha, &job.sha[..8]),
        if let Some(pr) = job.github_pr {
            format!(
                "<b>GitHub PR</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/pull/{}\">#{}</a>\n",
                pr, pr
            )
        } else {
            String::new()
        },
        job.arch,
        &job.packages.join(", "),
        &successful_packages.join(", "),
        &failed_package.clone().unwrap_or(String::from("None")),
        &skipped_packages.join(", "),
        if let Some(log) = log {
            Cow::Owned(format!("<a href=\"{}\">Build Log >></a>", log))
        } else {
            Cow::Borrowed("Failed to push log! See <code>/buildroots/buildit/buildit/push_failed_logs</code> to see log.")
        }
    )
}

pub fn to_markdown_build_result(job: &JobOk, success: bool) -> String {
    let JobOk {
        job,
        successful_packages,
        failed_package,
        skipped_packages,
        log,
        worker,
        elapsed,
        ..
    } = job;

    format!(
        "{} Job completed on {} \\({}\\)\n\n{}**Time elapsed**: {}\n{}**Architecture**: {}\n**Package\\(s\\) to build**: {}\n**Package\\(s\\) successfully built**: {}\n**Package\\(s\\) failed to build**: {}\n**Package\\(s\\) not built due to previous build failure**: {}\n\n{}\n",
        if success { SUCCESS } else { FAILED },
        worker.hostname,
        worker.arch,
        if let Some(enqueue_time) = &job.enqueue_time {
            format!("**Enqueue time**: {}\n", teloxide::utils::markdown::escape(&enqueue_time.to_string()))
        } else {
            String::new()
        },
        format_args!("{:.2?}", elapsed),
        format!("**Git commit**: [{}](https://github.com/AOSC-Dev/aosc-os-abbs/commit/{})\n", &job.sha[..8], job.sha),
        job.arch,
        teloxide::utils::markdown::escape(&job.packages.join(", ")),
        teloxide::utils::markdown::escape(&successful_packages.join(", ")),
        teloxide::utils::markdown::escape(&failed_package.clone().unwrap_or(String::from("None"))),
        teloxide::utils::markdown::escape(&skipped_packages.join(", ")),
        if let Some(log) = log {
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
fn test_format_html_new_job_summary() {
    let s = to_html_new_job_summary("fd-9.0.0", Some(4992), &["amd64"], &["fd".to_string()]);
    assert_eq!(s, "<b><u>New Job Summary</u></b>\n\n<b>Git reference</b>: fd-9.0.0\n<b>GitHub PR</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/pull/4992\">#4992</a>\n<b>Architecture(s)</b>: amd64\n<b>Package(s)</b>: fd")
}

#[test]
fn test_format_html_build_result() {
    use chrono::TimeZone;
    use common::{Job, JobOk, JobSource, WorkerIdentifier};
    use std::time::Duration;

    let job = JobOk {
        job: Job {
            packages: vec!["fd".to_string()],
            git_ref: "fd-9.0.0".to_string(),
            sha: "12345".to_string(),
            arch: "amd64".to_owned(),
            source: JobSource::Telegram(484493567),
            github_pr: Some(4992),
            noarch: false,
            enqueue_time: Some(
                chrono::Utc
                    .from_utc_datetime(&chrono::NaiveDateTime::from_timestamp_opt(61, 0).unwrap()),
            ),
        },
        successful_packages: vec!["fd".to_string()],
        failed_package: None,
        skipped_packages: vec![],
        log: Some("https://pastebin.aosc.io/paste/c0rWzj4EsSC~CVXs2qXtFw".to_string()),
        worker: WorkerIdentifier {
            arch: "amd64".to_string(),
            hostname: "Yerus".to_string(),
            pid: 54355,
        },
        elapsed: Duration::from_secs_f64(888.85),
        git_commit: Some("34acef168fc5ec454d3825fc864964951b130b49".to_string()),
        pushpkg_success: true,
    };

    let s = to_html_build_result(&job, true);

    assert_eq!(s, "✅\u{fe0f} Job completed on Yerus (amd64)\n\n<b>Enqueue time</b>: 1970-01-01 00:01:01 UTC\n<b>Time elapsed</b>: 888.85s\n<b>Git commit</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/commit/34acef168fc5ec454d3825fc864964951b130b49\">34acef16</a>\n<b>GitHub PR</b>: <a href=\"https://github.com/AOSC-Dev/aosc-os-abbs/pull/4992\">#4992</a>\n<b>Architecture</b>: amd64\n<b>Package(s) to build</b>: fd\n<b>Package(s) successfully built</b>: fd\n<b>Package(s) failed to build</b>: None\n<b>Package(s) not built due to previous build failure</b>: \n\n<a href=\"https://pastebin.aosc.io/paste/c0rWzj4EsSC~CVXs2qXtFw\">Build Log >></a>")
}
