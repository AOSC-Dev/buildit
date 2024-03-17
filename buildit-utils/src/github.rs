use anyhow::{anyhow, bail, Context};
use fancy_regex::Regex;
use gix::{
    prelude::ObjectIdExt, sec, sec::trust::DefaultForLevel, Repository, ThreadSafeRepository,
};
use jsonwebtoken::EncodingKey;
use octocrab::{models::pulls::PullRequest, params};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Output,
};
use tokio::{process, task};
use tracing::{debug, error, info};
use walkdir::WalkDir;

use crate::{
    ALL_ARCH, AMD64, ARM64, COMMITS_COUNT_LIMIT, LOONGARCH64, LOONGSON3, MIPS64R6EL, NOARCH,
    PPC64EL, RISCV64,
};

macro_rules! PR {
    () => {
        "Topic Description\n-----------------\n\n{}\n\nPackage(s) Affected\n-------------------\n\n{}\n\nSecurity Update?\n----------------\n\nNo\n\nBuild Order\n-----------\n\n```\n{}\n```\n\nTest Build(s) Done\n------------------\n\n{}"
    };
}

struct OpenPR<'a> {
    access_token: String,
    title: &'a str,
    head: &'a str,
    packages: &'a str,
    id: u64,
    key: EncodingKey,
    desc: &'a str,
    pkg_affected: &'a [String],
    tags: Option<&'a [String]>,
    archs: &'a [&'a str],
}

pub struct OpenPRRequest<'a> {
    pub git_ref: String,
    pub abbs_path: PathBuf,
    pub packages: String,
    pub title: String,
    pub tags: Option<Vec<String>>,
    pub archs: Vec<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenPRError {
    #[error(transparent)]
    IOError(#[from] tokio::io::Error),
    #[error(transparent)]
    Github(#[from] octocrab::Error),
    #[error(transparent)]
    Tokio(#[from] tokio::task::JoinError),
    #[error(transparent)]
    JsonWebToken(#[from] jsonwebtoken::errors::Error),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

pub async fn open_pr(
    app_private_key_path: &Path,
    access_token: &str,
    app_id: u64,
    openpr_request: OpenPRRequest<'_>,
) -> Result<String, OpenPRError> {
    let key = tokio::fs::read(app_private_key_path).await?;
    let key = tokio::task::spawn_blocking(move || jsonwebtoken::EncodingKey::from_rsa_pem(&key))
        .await??;

    let OpenPRRequest {
        git_ref,
        abbs_path,
        packages,
        title,
        tags,
        archs,
    } = openpr_request;

    update_abbs(&git_ref, &abbs_path).await?;
    let abbs_path_clone = abbs_path.clone();

    let commits = task::spawn_blocking(move || get_commits(abbs_path.as_ref())).await??;
    let commits = task::spawn_blocking(move || handle_commits(&commits)).await??;
    let pkgs = packages
        .split(',')
        .map(|x| x.to_string())
        .collect::<Vec<_>>();

    let pkg_affected =
        task::spawn_blocking(move || find_version_by_packages(&pkgs, &abbs_path_clone)).await??;

    let pr = open_pr_inner(OpenPR {
        access_token: access_token.to_string(),
        title: &title,
        head: &git_ref,
        packages: &packages,
        id: app_id,
        key: key.clone(),
        desc: &commits,
        pkg_affected: &pkg_affected,
        tags: tags.as_deref(),
        archs: &archs,
    })
    .await?;

    Ok(pr.html_url.map(|x| x.to_string()).unwrap_or_else(|| pr.url))
}

fn find_version_by_packages(pkgs: &[String], p: &Path) -> anyhow::Result<Vec<String>> {
    let mut res = vec![];

    let mut req_pkgs = vec![];

    for i in pkgs {
        if i.starts_with("groups/") {
            let f = fs::File::open(p.join(i))?;
            let lines = BufReader::new(f).lines();

            for i in lines {
                let i = i?;
                let pkg = i.split('/').next_back().unwrap_or(&i);
                req_pkgs.push(pkg.to_string());
            }
        } else {
            req_pkgs.push(i.to_string());
        }
    }

    for_each_abbs(p, |pkg, path| {
        if !req_pkgs.contains(&pkg.to_string()) {
            return;
        }

        let defines = path.join("autobuild").join("defines");
        let spec = path.join("spec");
        let spec = std::fs::read_to_string(spec);
        let defines = std::fs::read_to_string(defines);

        if let Ok(spec) = spec {
            let spec = read_ab_with_apml(&spec);
            let ver = spec.get("VER");
            let rel = spec.get("REL");
            let defines = defines.ok().map(|defines| read_ab_with_apml(&defines));

            let epoch = if let Some(ref def) = defines {
                def.get("PKGEPOCH")
            } else {
                None
            };

            if ver.is_none() {
                debug!("{pkg} has no VER variable");
            }

            let mut final_version = String::new();
            if let Some(epoch) = epoch {
                final_version.push_str(&format!("{epoch}:"));
            }

            final_version.push_str(ver.unwrap());

            if let Some(rel) = rel {
                final_version.push_str(&format!("-{rel}"));
            }

            res.push(format!("- {pkg}: {final_version}"));
        }
    });

    Ok(res)
}

/// Describe new commits for pull request
fn handle_commits(commits: &[Commit]) -> anyhow::Result<String> {
    let mut s = String::new();
    for (i, c) in commits.iter().enumerate() {
        if i == COMMITS_COUNT_LIMIT {
            let more = commits.len() - COMMITS_COUNT_LIMIT;
            if more > 0 {
                s.push_str(&format!("\n... and {more} more commits"));
            }
            break;
        }

        s.push_str(&format!("- {}\n", c.msg.0.trim()));
        if let Some(body) = &c.msg.1 {
            let body = body.split('\n');
            for line in body {
                let line = line.trim();
                if !line.is_empty() {
                    s.push_str(&format!("    {line}\n"));
                }
            }
        }
    }

    while s.ends_with('\n') {
        s.pop();
    }

    Ok(s)
}

struct Commit {
    _id: String,
    msg: (String, Option<String>),
}

/// Compute new commits on top of stable
fn get_commits(path: &Path) -> anyhow::Result<Vec<Commit>> {
    let mut res = vec![];
    let repo = get_repo(path)?;
    let commits = repo
        .head()?
        .try_into_peeled_id()?
        .ok_or(anyhow!("Failed to get peeled id"))?
        .ancestors()
        .all()?;

    let refrences = repo.references()?;
    let stable_branch = refrences
        .local_branches()?
        .filter_map(Result::ok)
        .find(|x| x.name().shorten() == "stable")
        .ok_or(anyhow!("failed to get stable branch"))?;

    // Collect commits on stable branch
    let commits_on_stable = stable_branch
        .into_fully_peeled_id()?
        .object()?
        .into_commit()
        .ancestors()
        .all()?;

    let mut commits_on_stable_set = HashSet::new();
    for i in commits_on_stable {
        let id = i?.id;
        commits_on_stable_set.insert(id);
    }

    // Collect commits on new branch, but not on stable branch
    // Mimic git log stable..HEAD
    for i in commits {
        let id = i?.id;
        if commits_on_stable_set.contains(&id) {
            continue;
        }

        let o = id.attach(&repo).object()?;
        let commit = o.into_commit();
        let commit_str = commit.id.to_string();

        let msg = commit.message()?;

        res.push(Commit {
            _id: commit_str,
            msg: (msg.title.to_string(), msg.body.map(|x| x.to_string())),
        })
    }

    Ok(res)
}

/// Update ABBS tree commit logs
#[tracing::instrument(skip(abbs_path))]
pub async fn update_abbs<P: AsRef<Path>>(git_ref: &str, abbs_path: P) -> anyhow::Result<()> {
    info!("Running git checkout -b stable ...");

    let abbs_path = abbs_path.as_ref();

    let output = process::Command::new("git")
        .arg("checkout")
        .arg("-b")
        .arg("stable")
        .current_dir(abbs_path)
        .output()
        .await?;

    print_stdout_and_stderr(&output);

    info!("Running git checkout stable ...");

    let output = process::Command::new("git")
        .arg("checkout")
        .arg("stable")
        .current_dir(abbs_path)
        .output()
        .await?;

    print_stdout_and_stderr(&output);

    info!("Running git pull ...");

    let output = process::Command::new("git")
        .arg("pull")
        .current_dir(abbs_path)
        .output()
        .await?;

    print_stdout_and_stderr(&output);

    info!("Running git fetch origin {git_ref} ...");

    let output = process::Command::new("git")
        .arg("fetch")
        .arg("origin")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .await?;

    print_stdout_and_stderr(&output);

    if !output.status.success() {
        bail!("Failed to fetch origin git-ref: {git_ref}");
    }

    info!("Running git checkout -b {git_ref} ...");

    let output = process::Command::new("git")
        .arg("checkout")
        .arg("-b")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .await?;

    print_stdout_and_stderr(&output);

    info!("Running git checkout {git_ref} ...");

    let output = process::Command::new("git")
        .arg("checkout")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .await?;

    print_stdout_and_stderr(&output);

    if !output.status.success() {
        bail!("Failed to checkout {git_ref}");
    }

    info!("Running git reset FETCH_HEAD --hard ...");

    let output = process::Command::new("git")
        .args(["reset", "FETCH_HEAD", "--hard"])
        .current_dir(abbs_path)
        .output()
        .await?;

    print_stdout_and_stderr(&output);

    if !output.status.success() {
        bail!("Failed to checkout {git_ref}");
    }

    Ok(())
}

fn print_stdout_and_stderr(output: &Output) {
    info!("Output:");
    info!("  Stdout:");
    info!(" {}", String::from_utf8_lossy(&output.stdout));
    info!("  Stderr:");
    info!(" {}", String::from_utf8_lossy(&output.stderr));
}

pub fn get_repo(path: &Path) -> anyhow::Result<Repository> {
    let mut git_open_opts_map = sec::trust::Mapping::<gix::open::Options>::default();

    let config = gix::open::permissions::Config {
        git_binary: false,
        system: false,
        git: false,
        user: false,
        env: true,
        includes: true,
    };

    git_open_opts_map.reduced = git_open_opts_map
        .reduced
        .permissions(gix::open::Permissions {
            config,
            ..gix::open::Permissions::default_for_level(sec::Trust::Reduced)
        });

    git_open_opts_map.full = git_open_opts_map.full.permissions(gix::open::Permissions {
        config,
        ..gix::open::Permissions::default_for_level(sec::Trust::Full)
    });

    let shared_repo = ThreadSafeRepository::discover_with_environment_overrides_opts(
        path,
        Default::default(),
        git_open_opts_map,
    )
    .context("Failed to find git repo")?;

    let repository = shared_repo.to_thread_local();

    Ok(repository)
}

/// Open Pull Request
async fn open_pr_inner(pr: OpenPR<'_>) -> Result<PullRequest, octocrab::Error> {
    let OpenPR {
        access_token,
        title,
        head,
        packages,
        id,
        key,
        desc,
        pkg_affected,
        tags,
        archs,
    } = pr;

    let crab = octocrab::Octocrab::builder()
        .app(id.into(), key)
        .user_access_token(access_token)
        .build()?;

    // pr body
    let body = format!(
        PR!(),
        desc,
        pkg_affected.join("\n"),
        format!("#buildit {}", packages.replace(',', " ")),
        format_archs(archs)
    );

    // pr tags
    let tags = if let Some(tags) = tags {
        Cow::Borrowed(tags)
    } else {
        Cow::Owned(auto_add_label(title))
    };

    // check if there are existing open pr

    let page = crab
        .pulls("AOSC-Dev", "aosc-os-abbs")
        .list()
        // Optional Parameters
        .state(params::State::Open)
        .head(format!("AOSC-Dev:{}", head))
        .base("stable")
        // Send the request
        .send()
        .await?;

    for old_pr in page.items {
        if old_pr.head.ref_field == head {
            // double check

            // update existing pr
            let pr = crab
                .pulls("AOSC-Dev", "aosc-os-abbs")
                .update(old_pr.number)
                .title(title)
                .body(&body)
                .send()
                .await?;

            if !tags.is_empty() {
                crab.issues("AOSC-Dev", "aosc-os-abbs")
                    .add_labels(pr.number, &tags)
                    .await?;
            }

            return Ok(pr);
        }
    }

    // create a new pr
    let pr = crab
        .pulls("AOSC-Dev", "aosc-os-abbs")
        .create(title, head, "stable")
        .draft(false)
        .maintainer_can_modify(true)
        .body(&body)
        .send()
        .await?;

    if !tags.is_empty() {
        crab.issues("AOSC-Dev", "aosc-os-abbs")
            .add_labels(pr.number, &tags)
            .await?;
    }

    Ok(pr)
}

/// Add labels based on pull request title
fn auto_add_label(title: &str) -> Vec<String> {
    let mut labels = vec![];
    let title = title
        .to_ascii_lowercase()
        .split_ascii_whitespace()
        .map(|x| {
            x.chars()
                .filter(|x| x.is_ascii_alphabetic() || x.is_ascii_alphanumeric())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ");

    let v = vec![
        ("fix", vec![String::from("has-fix")]),
        ("update", vec![String::from("upgrade")]),
        ("upgrade", vec![String::from("upgrade")]),
        ("downgrade", vec![String::from("downgrade")]),
        ("survey", vec![String::from("survey")]),
        ("drop", vec![String::from("drop-package")]),
        ("security", vec![String::from("security")]),
        ("cve", vec![String::from("security")]),
        ("0day", vec![String::from("0day"), String::from("security")]),
        ("improve", vec![String::from("enhancement")]),
        ("enhance", vec![String::from("enhancement")]),
        ("dep", vec![String::from("dependencies")]),
        ("dependencies", vec![String::from("dependencies")]),
        ("dependency", vec![String::from("dependencies")]),
        ("pkgdep", vec![String::from("dependencies")]),
        ("builddep", vec![String::from("dependencies")]),
        ("depend", vec![String::from("dependencies")]),
        ("core", vec![String::from("core")]),
        ("mips64r6el", vec![String::from("cip-pilot")]),
        ("mipsisa64r6el", vec![String::from("cip-pilot")]),
        ("mipsr6", vec![String::from("cip-pilot")]),
        ("r6", vec![String::from("cip-pilot")]),
        ("linux-kernel", vec![String::from("kernel")]),
        ("new", vec![String::from("new-packages")]),
        ("preview", vec![String::from("preview")]),
        (
            "ftbfs",
            vec![String::from("has-fix"), String::from("ftbfs")],
        ),
        ("rework", vec![String::from("rework")]),
    ];

    for (k, v) in v {
        if title.contains(k) {
            labels.extend(v);
        }
    }

    // de-duplicate
    let mut res = vec![];
    for i in labels {
        if res.contains(&i) {
            continue;
        }

        res.push(i);
    }

    res
}

fn format_archs(archs: &[&str]) -> String {
    let mut s = "".to_string();

    let mut map = HashMap::new();
    map.insert("amd64", AMD64);
    map.insert("arm64", ARM64);
    map.insert("noarch", NOARCH);
    map.insert("loongarch64", LOONGARCH64);
    map.insert("loongson3", LOONGSON3);
    map.insert("mips64r6el", MIPS64R6EL);
    map.insert("ppc64el", PPC64EL);
    map.insert("riscv64", RISCV64);

    let mut newline = false;

    // Primary Architectures
    if archs.contains(&"amd64")
        || archs.contains(&"arm64")
        || archs.contains(&"loongarch64")
        || archs.contains(&"noarch")
    {
        s.push_str("**Primary Architectures**\n\n");
        newline = true;
    }

    for i in ["amd64", "arm64", "loongarch64", "noarch"] {
        if archs.contains(&i) {
            s.push_str(&format!("- [ ] {}\n", map[i]));
        }
    }

    // Secondary Architectures
    if archs.contains(&"loongson3") || archs.contains(&"ppc64el") || archs.contains(&"riscv64") {
        if newline {
            s.push('\n');
        }
        s.push_str("**Secondary Architectures**\n\n");
        newline = true;
    }

    for i in ["loongson3", "ppc64el", "riscv64"] {
        if archs.contains(&i) {
            s.push_str(&format!("- [ ] {}\n", map[i]));
        }
    }

    // Experimental Architectures
    if archs.contains(&"mips64r6el") {
        if newline {
            s.push('\n');
        }
        s.push_str("**Experimental Architectures**\n\n");
    }

    for i in ["mips64r6el"] {
        if archs.contains(&i) {
            s.push_str(&format!("- [ ] {}\n", map[i]));
        }
    }

    s
}

pub fn get_archs<'a>(p: &'a Path, packages: &'a [String]) -> Vec<&'a str> {
    let mut is_noarch = vec![];
    let mut fail_archs = vec![];

    // strip modifiers, e.g. gmp:+stage2 becomes gmp
    let packages: Vec<String> = packages
        .iter()
        .map(|s| {
            (match s.split_once(":") {
                Some((prefix, _suffix)) => prefix,
                None => s,
            })
            .to_string()
        })
        .collect();

    for_each_abbs(p, |pkg, path| {
        if !packages.contains(&pkg.to_string()) {
            return;
        }

        let defines_list = if path.join("autobuild").exists() {
            vec![path.join("autobuild").join("defines")]
        } else {
            let mut defines_list = vec![];
            for i in WalkDir::new(path)
                .max_depth(1)
                .min_depth(1)
                .into_iter()
                .flatten()
            {
                if !i.path().is_dir() {
                    continue;
                }
                let defines_path = i.path().join("defines");
                if defines_path.exists() {
                    defines_list.push(defines_path);
                }
            }

            defines_list
        };

        for i in defines_list {
            let defines = std::fs::read_to_string(i);

            if let Ok(defines) = defines {
                let defines = read_ab_with_apml(&defines);

                is_noarch.push(
                    defines
                        .get("ABHOST")
                        .map(|x| x == "noarch")
                        .unwrap_or(false),
                );

                if let Some(fail_arch) = defines.get("FAIL_ARCH") {
                    fail_archs.push(fail_arch_regex(fail_arch).ok())
                } else {
                    fail_archs.push(None);
                };
            }
        }
    });

    if is_noarch.is_empty() || is_noarch.iter().any(|x| !x) {
        if fail_archs.is_empty() {
            return ALL_ARCH.iter().map(|x| x.to_owned()).collect();
        }

        if fail_archs.iter().any(|x| x.is_none()) {
            ALL_ARCH.iter().map(|x| x.to_owned()).collect()
        } else {
            let mut res = vec![];

            for i in fail_archs {
                let r = i.unwrap();
                for a in ALL_ARCH.iter().map(|x| x.to_owned()) {
                    if !r.is_match(a).unwrap_or(false) && !res.contains(&a) {
                        res.push(a);
                    }
                }
            }

            res
        }
    } else {
        vec!["noarch"]
    }
}

pub fn read_ab_with_apml(file: &str) -> HashMap<String, String> {
    let mut context = HashMap::new();

    // Try to set some ab3 flags to reduce the chance of returning errors
    for i in ["ARCH", "PKGDIR", "SRCDIR"] {
        context.insert(i.to_string(), "".to_string());
    }

    match abbs_meta_apml::parse(file, &mut context).map_err(|e| {
        let e: Vec<String> = e.iter().map(|e| e.to_string()).collect();
        anyhow!(e.join("; "))
    }) {
        Ok(()) => (),
        Err(e) => {
            error!("{e}, buildit will use fallback method to parse file");
            for line in file.split('\n') {
                let stmt = line.split_once('=');
                if let Some((name, value)) = stmt {
                    context.insert(name.to_string(), value.replace('\"', ""));
                }
            }
        }
    };

    context
}

pub fn for_each_abbs<F: FnMut(&str, &Path)>(path: &Path, mut f: F) {
    for i in WalkDir::new(path)
        .max_depth(2)
        .min_depth(2)
        .into_iter()
        .flatten()
    {
        if i.path().is_file() {
            continue;
        }

        let pkg = i.file_name().to_str();

        if pkg.is_none() {
            debug!("Failed to convert str: {}", i.path().display());
            continue;
        }

        let pkg = pkg.unwrap();

        f(pkg, i.path());
    }
}

pub fn fail_arch_regex(expr: &str) -> anyhow::Result<Regex> {
    let mut regex = String::from("^");
    let mut negated = false;
    let mut sup_bracket = false;

    if expr.len() < 3 {
        return Err(anyhow!("Pattern too short."));
    }

    let expr = expr.as_bytes();
    for (i, c) in expr.iter().enumerate() {
        if i == 0 && c == &b'!' {
            negated = true;
            if expr.get(1) != Some(&b'(') {
                regex += "(";
                sup_bracket = true;
            }
            continue;
        }
        if negated {
            if c == &b'(' {
                regex += "(?!";
                continue;
            } else if i == 1 && sup_bracket {
                regex += "?!";
            }
        }
        regex += std::str::from_utf8(&[*c])?;
    }

    if sup_bracket {
        regex += ")";
    }

    Ok(Regex::new(&regex)?)
}

#[test]
fn test_get_archs() {
    let binding = ["autobuild3".to_owned(), "autobuild4".to_owned()];
    let a = get_archs(Path::new("/home/saki/aosc-os-abbs"), &binding);

    assert_eq!(
        a,
        vec![
            "amd64",
            "arm64",
            "loongarch64",
            "loongson3",
            "mips64r6el",
            "ppc64el",
            "riscv64",
        ]
    );
}

#[test]
fn test_auto_add_label() {
    let title = "266: update to 114514";
    let s = auto_add_label(title);
    assert_eq!(s, vec!["upgrade".to_string()]);

    let title = "266: security update to 114514";
    let s = auto_add_label(title);
    assert_eq!(s, vec!["upgrade".to_string(), "security".to_string()]);

    let title = "266: fix 0day";
    let s = auto_add_label(title);
    assert_eq!(
        s,
        vec![
            "has-fix".to_string(),
            "0day".to_string(),
            "security".to_string()
        ]
    );
}
