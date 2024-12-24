use crate::github::{find_version_by_packages, print_stdout_and_stderr, update_abbs};
use abbs_update_checksum_core::{get_new_spec, ParseErrors};
use anyhow::{bail, Context};
use github::{for_each_abbs, get_spec};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
};
use tokio::{fs, task::spawn_blocking};
use tracing::{debug, error, info, warn};

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
                                if let Err(e) = update_version(&version, path, true) {
                                    res = Err(e.into());
                                    return;
                                }
                                is_upstream_ver = true;
                            }
                        }

                        if !is_upstream_ver {
                            for line in lines {
                                if line.starts_with("VER") {
                                    if let Err(e) = update_version(&version, path, false) {
                                        res = Err(e.into());
                                        return;
                                    }
                                }
                            }
                        }

                        if let Err(e) = std::fs::write(spec, f) {
                            res = Err(e.into());
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
                .current_dir(&abbs_path)
                .output()
                .context("Running aosc-findupdate")?;

            print_stdout_and_stderr(&output);
        }
    }

    let status = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(&abbs_path)
        .output()
        .context("Finding modified files using git")?;

    let status = BufReader::new(&*status.stdout).lines().flatten().next();

    if let Some(status) = status {
        let split_status = status.trim().split_once(" ");
        if let Some((status, _)) = split_status {
            if status != "M" {
                bail!("{pkg} has no update");
            }

            let absolute_abbs_path = std::fs::canonicalize(abbs_path)?;
            let pkg_shared = pkg.to_owned();

            info!("Writting new checksum ...");
            let res = write_new_spec(absolute_abbs_path, pkg_shared).await;

            if let Err(e) = res {
                // cleanup repo
                Command::new("git")
                    .arg("reset")
                    .arg("HEAD")
                    .arg("--hard")
                    .current_dir(&abbs_path)
                    .output()
                    .context("Reset git repo status")?;

                bail!("Failed to run acbs-build to update checksum: {}", e);
            }

            let ver = find_version_by_packages(&[pkg.to_string()], &abbs_path)
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

            Command::new("git")
                .arg("branch")
                .arg("-f")
                .arg(&branch)
                .arg("stable")
                .current_dir(&abbs_path)
                .output()
                .context("Point new branch at stable")?;
            Command::new("git")
                .arg("checkout")
                .arg(&branch)
                .current_dir(&abbs_path)
                .output()
                .context("Checking out to the new branch")?;
            Command::new("git")
                .arg("add")
                .arg(".")
                .current_dir(&abbs_path)
                .output()
                .context("Staging modified files")?;
            Command::new("git")
                .arg("commit")
                .arg("-m")
                .arg(format!("{}\n\nCo-authored-by: {}", title, coauthor))
                .current_dir(&abbs_path)
                .output()
                .context("Creating git commit")?;
            Command::new("git")
                .arg("push")
                .arg("--set-upstream")
                .arg("origin")
                .arg(&branch)
                .arg("--force")
                .current_dir(&abbs_path)
                .output()
                .context("Pushing new commit to GitHub")?;

            return Ok(FindUpdate {
                package: pkg.to_string(),
                branch,
                title,
            });
        }
    }

    bail!("{pkg} has no update")
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
        .arg(&abbs_path_shared.join("acbs-log"))
        .arg("--cache-dir")
        .arg(&abbs_path_shared.join("acbs-cache"))
        .arg("--temp-dir")
        .arg(&abbs_path_shared.join("acbs-temp"))
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
