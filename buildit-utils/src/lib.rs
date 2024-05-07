use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::Command,
};

use anyhow::{bail, Context};
use walkdir::WalkDir;

pub mod github;

pub const AMD64: &str = "AMD64 `amd64`";
pub const ARM64: &str = "AArch64 `arm64`";
pub const NOARCH: &str = "Architecture-independent `noarch`";
pub const LOONGARCH64: &str = "LoongArch 64-bit `loongarch64`";
pub const LOONGSON3: &str = "Loongson 3 `loongson3`";
pub const MIPS64R6EL: &str = "MIPS R6 64-bit (Little Endian) `mips64r6el`";
pub const PPC64EL: &str = "PowerPC 64-bit (Little Endian) `ppc64el`";
pub const RISCV64: &str = "RISC-V 64-bit `riscv64`";
pub const COMMITS_COUNT_LIMIT: usize = 10;

pub(crate) const ALL_ARCH: &[&str] = &[
    "amd64",
    "arm64",
    "loongarch64",
    "loongson3",
    "mips64r6el",
    "ppc64el",
    "riscv64",
];

pub struct FindUpdate {
    pub package: String,
    pub branch: String,
    pub title: String,
}

pub fn find_update_and_update_checksum(pkg: &str, abbs_path: &Path) -> anyhow::Result<FindUpdate> {
    Command::new("aosc-findupdate")
        .arg("-i")
        .arg(format!("^{pkg}$"))
        .output()?;

    let status = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .output()?;

    let status = BufReader::new(&*status.stdout).lines().flatten().next();

    if let Some(status) = status {
        let split_status = status.split_once(" ");
        if let Some((status, _)) = split_status {
            if status != "M" {
                bail!("{pkg} has no update");
            }

            Command::new("ciel")
                .arg("shell")
                .arg("-i")
                .arg("main")
                .arg("acbs-build")
                .arg("-gw")
                .arg(pkg)
                .output()?;

            let path = std::env::current_dir()?;
            std::env::set_current_dir(abbs_path)?;

            let mut ver = None;

            for i in WalkDir::new(".").max_depth(3).min_depth(3) {
                let i = i?;
                if i.file_name() == "spec" {
                    let f = std::fs::File::open(i.path())?;
                    let line = BufReader::new(f)
                        .lines()
                        .flatten()
                        .next()
                        .context(format!("Failed to open file: {}", i.path().display()))?;
                    let (_, v) = line.split_once('=').context(format!("Failed to open file: {}", i.path().display()))?;
                    ver = Some(v.trim().to_string());
                }
            }

            let ver = ver.context(format!("Failed to find pkg version: {}", pkg))?;
            let branch = format!("{pkg}-{ver}");
            let title = format!("{pkg}: update to {ver}");

            Command::new("git").arg("checkout").arg("-b").arg(&branch).output()?;
            Command::new("git").arg("add").arg(".").output()?;
            Command::new("git")
                .arg("commit")
                .arg("-m")
                .arg(&title)
                .output()?;
            Command::new("git").arg("push").arg("--set-upstream").arg("origin").arg(&branch).output()?;
            std::env::set_current_dir(path)?;

            return Ok(FindUpdate { package: pkg.to_string(), branch, title });
        }
    }

    bail!("{pkg} has no update")
}
