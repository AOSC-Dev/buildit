use crate::github::{find_version_by_packages, print_stdout_and_stderr, update_abbs};
use abbs_update_checksum_core::{ParseErrors, get_new_spec};
use anyhow::{Context, bail};
use github::{for_each_abbs, get_spec};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Output,
};
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    task::spawn_blocking,
};
use tracing::{error, info, warn};

pub mod github;

pub const AMD64: &str = "AMD64 `amd64`";
pub const ARM64: &str = "AArch64 `arm64`";
pub const NOARCH: &str = "Architecture-independent `noarch`";
pub const LOONGARCH64: &str = "LoongArch 64-bit `loongarch64`";
pub const LOONGSON3: &str = "Loongson 3 `loongson3`";
pub const PPC64EL: &str = "PowerPC 64-bit (Little Endian) `ppc64el`";
pub const RISCV64: &str = "RISC-V 64-bit `riscv64`";
pub const COMMITS_COUNT_LIMIT: usize = 10;

pub(crate) const ALL_ARCH: &[&str] = &[
    "amd64",
    "arm64",
    "loongarch64",
    "loongson3",
    "ppc64el",
    "riscv64",
];

pub static ABBS_REPO_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

pub struct FindUpdate {
    pub package: String,
    pub branch: String,
    pub title: String,
}

fn update_version<P: AsRef<Path>>(
    new: &str,
    spec: P,
    replace_upstream_ver: bool,
) -> anyhow::Result<()> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(spec.as_ref())?;
    let mut content = String::new();
    f.read_to_string(&mut content)?;
    let replace_rel = Regex::new("REL=.+\\s+").unwrap();

    let replaced = if replace_upstream_ver {
        let replace = Regex::new("UPSTREAM_VER=.+").unwrap();
        replace.replace(&content, format!("UPSTREAM_VER={}", new))
    } else {
        let replace = Regex::new("VER=.+").unwrap();
        replace.replace(&content, format!("VER={}", new))
    };
    let replaced = replace_rel.replace(&replaced, "");

    f.seek(SeekFrom::Start(0))?;
    let bytes = replaced.as_bytes();
    f.write_all(bytes)?;
    f.set_len(bytes.len() as u64)?;

    Ok(())
}

#[tracing::instrument(skip(abbs_path))]
pub async fn find_update_and_update_checksum(
    pkg: &str,
    abbs_path: &Path,
    coauthor: &str,
    manual_update: Option<&str>,
) -> anyhow::Result<FindUpdate> {
    let _lock = ABBS_REPO_LOCK.lock().await;

    // switch to stable branch
    update_abbs("stable", &abbs_path, false).await?;

    match manual_update {
        Some(version) => {
            info!("manual version: {version}");
            let pkg: Box<str> = Box::from(pkg);
            let version = Box::from(version);
            let abbs_path: Box<Path> = Box::from(abbs_path);
            tokio::task::spawn_blocking(move || {
                let mut res: anyhow::Result<()> = Ok(());
                for_each_abbs(&abbs_path, |for_each_pkg, path| {
                    if *for_each_pkg == *pkg {
                        let spec = path.join("spec");
                        let f = std::fs::read_to_string(&spec);
                        let f = match f {
                            Ok(f) => f,
                            Err(e) => {
                                res = Err(e.into());
                                return;
                            }
                        };
                        let lines: Vec<Box<str>> = f.lines().map(Box::from).collect::<Vec<_>>();
                        let mut is_upstream_ver = false;
                        for line in &lines {
                            if line.starts_with("UPSTREAM_VER") {
                                if let Err(e) = update_version(&version, &spec, true) {
                                    res = Err(e);
                                    return;
                                }
                                is_upstream_ver = true;
                            }
                        }

                        if !is_upstream_ver {
                            for line in lines {
                                if line.starts_with("VER") {
                                    if let Err(e) = update_version(&version, &spec, false) {
                                        res = Err(e);
                                        return;
                                    }
                                }
                            }
                        }
                    }
                });

                res
            })
            .await??;
        }
        None => {
            info!("Running aosc-findupdate ...");

            let output = Command::new("aosc-findupdate")
                .arg("-i")
                .arg(format!(".*/{pkg}$"))
                .current_dir(abbs_path)
                .output()
                .await
                .context("Running aosc-findupdate")?;

            print_stdout_and_stderr(&output);
        }
    }

    let status = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(abbs_path)
        .output()
        .await
        .context("Finding modified files using git")?;

    let status = BufReader::new(&*status.stdout).lines().next_line().await;

    if let Ok(Some(status)) = status {
        let split_status = status.trim().split_once(" ");
        if let Some((status, _)) = split_status {
            match git_push(status, pkg, abbs_path, coauthor).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    git_reset(abbs_path).await?;
                    return Err(e);
                }
            }
        }
    }

    bail!("{pkg} has no update")
}

async fn git_push(
    status: &str,
    pkg: &str,
    abbs_path: &Path,
    coauthor: &str,
) -> Result<FindUpdate, anyhow::Error> {
    if status != "M" {
        bail!("{pkg} has no update");
    }

    let absolute_abbs_path = std::fs::canonicalize(abbs_path)?;
    let pkg_shared = pkg.to_owned();

    info!("Writing new checksum ...");
    write_new_spec(absolute_abbs_path, pkg_shared)
        .await
        .context("Failed to run acbs-build to update checksum")?;

    let ver = find_version_by_packages(&[pkg.to_string()], abbs_path)
        .into_iter()
        .next();

    let mut ver = ver
        .context(format!("Failed to find pkg version: {}", pkg))?
        .1;

    // skip epoch
    if let Some((_prefix, suffix)) = ver.split_once(':') {
        ver = suffix.to_string();
    }

    let branch = format!("{pkg}-{ver}");
    let title = format!("{pkg}: update to {ver}");

    let branches = Command::new("git").arg("branch").output().await?;
    let mut branches_stdout = BufReader::new(&*branches.stdout).lines();

    while let Ok(Some(line)) = branches_stdout.next_line().await {
        if line.contains(&branch) {
            bail!("Branch {} already exists.", branch);
        }
    }

    Command::new("git")
        .arg("branch")
        .arg("-f")
        .arg(&branch)
        .arg("stable")
        .current_dir(abbs_path)
        .output()
        .await
        .context("Point new branch at stable")?;
    Command::new("git")
        .arg("checkout")
        .arg(&branch)
        .current_dir(abbs_path)
        .output()
        .await
        .context("Checking out to the new branch")?;
    Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(abbs_path)
        .output()
        .await
        .context("Staging modified files")?;
    Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg(format!("{}\n\nCo-authored-by: {}", title, coauthor))
        .current_dir(abbs_path)
        .output()
        .await
        .context("Creating git commit")?;
    Command::new("git")
        .arg("push")
        .arg("--set-upstream")
        .arg("origin")
        .arg(&branch)
        .arg("--force")
        .current_dir(abbs_path)
        .output()
        .await
        .context("Pushing new commit to GitHub")?;

    Ok(FindUpdate {
        package: pkg.to_string(),
        branch,
        title,
    })
}

async fn write_new_spec(abbs_path: PathBuf, pkg: String) -> anyhow::Result<()> {
    let pkg_shared = pkg.clone();
    let abbs_path_shared = abbs_path.clone();
    let (mut spec, p) = spawn_blocking(move || get_spec(&abbs_path_shared, &pkg_shared)).await??;

    for i in 1..=5 {
        match get_new_spec(&mut spec, |_, _, _, _| {}, 4).await {
            Ok(()) => {
                if i > 1 {
                    warn!("({i}/5) Retrying to get new spec...");
                }

                fs::write(p, spec).await?;
                return Ok(());
            }
            Err(e) => {
                if let Some(e) = e.downcast_ref::<ParseErrors>() {
                    warn!("{e}, try use acbs-build fallback to get new checksum ...");
                    acbs_build_gw(&pkg, &abbs_path).await?;
                } else {
                    error!("Failed to get new spec: {e}");
                    if i == 5 {
                        bail!("{e}");
                    }
                }
            }
        }
    }

    Ok(())
}

async fn acbs_build_gw(pkg_shared: &str, abbs_path_shared: &Path) -> anyhow::Result<()> {
    let output = tokio::process::Command::new("acbs-build")
        .arg("-gw")
        .arg(pkg_shared)
        .arg("--log-dir")
        .arg(abbs_path_shared.join("acbs-log"))
        .arg("--cache-dir")
        .arg(abbs_path_shared.join("acbs-cache"))
        .arg("--temp-dir")
        .arg(abbs_path_shared.join("acbs-temp"))
        .arg("--tree-dir")
        .arg(abbs_path_shared)
        .current_dir(abbs_path_shared)
        .output()
        .await
        .context("Running acbs-build to update checksums")?;

    print_stdout_and_stderr(&output);

    if !output.status.success() {
        bail!(
            "Failed to run acbs-build to update checksum: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

async fn git_reset(abbs_path: &Path) -> Result<Output, anyhow::Error> {
    Command::new("git")
        .arg("reset")
        .arg("HEAD")
        .arg("--hard")
        .current_dir(abbs_path)
        .output()
        .await
        .context("Reset git repo status")
}
