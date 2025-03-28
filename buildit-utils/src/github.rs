use anyhow::{anyhow, bail, Context};
use fancy_regex::Regex;
use gix::{
    prelude::ObjectIdExt, sec, sec::trust::DefaultForLevel, Repository, ThreadSafeRepository,
};
use jsonwebtoken::EncodingKey;
use octocrab::{models::pulls::PullRequest, params};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Output,
};
use tokio::{process, task};
use tracing::{debug, error, info, info_span, warn, Instrument};
use walkdir::WalkDir;

use crate::{
    ABBS_REPO_LOCK, ALL_ARCH, AMD64, ARM64, COMMITS_COUNT_LIMIT, LOONGARCH64, LOONGSON3, NOARCH,
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

#[derive(Debug)]
pub struct OpenPRRequest<'a> {
    pub git_ref: String,
    pub abbs_path: PathBuf,
    pub packages: String,
    pub title: String,
    pub tags: Option<Vec<String>>,
    /// If None, automatically deduced via `get_archs()`
    pub archs: Option<Vec<&'a str>>,
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

// return (pr number, pr url)
#[tracing::instrument(skip(app_private_key_path, access_token, app_id))]
pub async fn open_pr(
    app_private_key_path: &Path,
    access_token: &str,
    app_id: u64,
    openpr_request: OpenPRRequest<'_>,
) -> Result<(u64, String), OpenPRError> {
    let key = tokio::fs::read(app_private_key_path).await?;
    let key = tokio::task::spawn_blocking(move || jsonwebtoken::EncodingKey::from_rsa_pem(&key))
        .await??;

    let OpenPRRequest {
        git_ref,
        abbs_path,
        packages,
        mut title,
        tags,
        archs,
    } = openpr_request;

    let _lock = ABBS_REPO_LOCK.lock().await;

    update_abbs(&git_ref, &abbs_path, false).await?;

    let abbs_path_clone = abbs_path.clone();
    let commits = task::spawn_blocking(move || get_commits(&abbs_path_clone))
        .instrument(info_span!("get_commits"))
        .await??;

    if title.is_empty() && commits.len() == 1 {
        // try to generate title
        title = commits[0].msg.0.to_owned();
    }

    if title.is_empty() {
        return Err(OpenPRError::Anyhow(anyhow!("PR title cannot be empty")));
    }

    let commits = task::spawn_blocking(move || handle_commits(&commits))
        .instrument(info_span!("handle_commits"))
        .await??;
    let pkgs = packages
        .split(',')
        .map(|x| x.to_string())
        .collect::<Vec<_>>();

    // handle modifiers and groups
    let resolved_pkgs = resolve_packages(&pkgs, &abbs_path)?;

    // deduce archs if not specified
    let archs = match archs {
        Some(archs) => archs,
        None => {
            let resolved_pkgs_clone = resolved_pkgs.clone();
            let abbs_path_clone = abbs_path.clone();
            task::spawn_blocking(move || get_archs(&abbs_path_clone, &resolved_pkgs_clone))
                .instrument(info_span!("get_archs"))
                .await?
        }
    };

    let abbs_path_clone = abbs_path.clone();
    let pkg_affected = task::spawn_blocking(move || {
        find_version_by_packages_list(&resolved_pkgs, &abbs_path_clone)
    })
    .instrument(info_span!("find_version_by_packages_list"))
    .await?;

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

    Ok((
        pr.number,
        pr.html_url.map(|x| x.to_string()).unwrap_or_else(|| pr.url),
    ))
}

/// `packages` should have no groups nor modifiers
/// return list of (package_name, version)
#[tracing::instrument(skip(p))]
pub fn find_version_by_packages(pkgs: &[String], p: &Path) -> Vec<(String, String)> {
    let mut res = vec![];

    for_each_abbs(p, |pkg, path| {
        if !pkgs.contains(&pkg.to_string()) {
            return;
        }

        let spec = path.join("spec");
        let spec = std::fs::read_to_string(spec);
        let defines_list = locate_defines(path);

        if let Ok(spec) = spec {
            let spec = read_ab_with_apml(&spec);
            let ver = spec.get("VER");
            let rel = spec.get("REL");
            if ver.is_none() {
                warn!("{pkg} has no VER variable");
                return;
            }

            for i in defines_list {
                if let Ok(defines) = std::fs::read_to_string(i) {
                    let defines = read_ab_with_apml(&defines);

                    if let Some(pkgname) = defines.get("PKGNAME") {
                        let epoch = defines.get("PKGEPOCH");

                        let mut final_version = String::new();
                        if let Some(epoch) = epoch {
                            final_version.push_str(&format!("{epoch}:"));
                        }

                        final_version.push_str(ver.unwrap());

                        if let Some(rel) = rel {
                            final_version.push_str(&format!("-{rel}"));
                        }

                        res.push((pkgname.clone(), final_version));
                    } else {
                        warn!("{pkg} has no PKGNAME variable");
                    }
                }
            }
        }
    });

    res.sort();

    res
}

/// `packages` should have no groups nor modifiers
#[tracing::instrument(skip(p))]
fn find_version_by_packages_list(pkgs: &[String], p: &Path) -> Vec<String> {
    let mut res = vec![];

    for (name, version) in find_version_by_packages(pkgs, p) {
        res.push(format!("- {name}: {version}"));
    }

    res
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

        s.push_str(&format!("- {}\n", escape(c.msg.0.trim())));
        if let Some(body) = &c.msg.1 {
            let body = body.split('\n');
            for line in body {
                let line = line.trim();
                if !line.is_empty() {
                    s.push_str(&format!("    {}\n", escape(line.trim())));
                }
            }
        }
    }

    while s.ends_with('\n') {
        s.pop();
    }

    Ok(s)
}

fn escape(text: &str) -> String {
    // the escaped strings only live for a short time, and they are short
    // so the waste of memeory are ignorable
    let mut result = String::with_capacity(text.len() * 2);
    for char in text.chars() {
        if matches!(char, '*' | '~' | '`' | '[' | ']' | '(' | ')' | '\\') {
            result.push('\\');
        }
        result.push(char);
    }
    result
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

    let references = repo.references()?;
    let stable_branch = references
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
pub async fn update_abbs<P: AsRef<Path>>(
    git_ref: &str,
    abbs_path: P,
    skip_git_fetch: bool,
) -> anyhow::Result<()> {
    info!("Running git checkout -b stable ...");

    let abbs_path = abbs_path.as_ref();

    let output = process::Command::new("git")
        .arg("checkout")
        .arg("-b")
        .arg("stable")
        .current_dir(abbs_path)
        .output()
        .instrument(info_span!("git_checkout_to_stable"))
        .await?;

    print_stdout_and_stderr(&output);

    info!("Running git checkout stable ...");

    let output = process::Command::new("git")
        .arg("checkout")
        .arg("stable")
        .current_dir(abbs_path)
        .output()
        .instrument(info_span!("git_checkout_to_stable"))
        .await?;

    print_stdout_and_stderr(&output);

    if skip_git_fetch {
        info!("Skippping git fetch ...")
    } else {
        info!("Running git fetch origin {git_ref} ...");

        let output = process::Command::new("git")
            .arg("fetch")
            .arg("origin")
            .arg(git_ref)
            .current_dir(abbs_path)
            .output()
            .instrument(info_span!("git_fetch_origin"))
            .await?;

        print_stdout_and_stderr(&output);

        if !output.status.success() {
            bail!("Failed to fetch origin git-ref: {git_ref}");
        }
    }

    info!("Running git reset origin/stable --hard ...");

    let output = process::Command::new("git")
        .arg("reset")
        .arg("origin/stable")
        .arg("--hard")
        .current_dir(abbs_path)
        .output()
        .instrument(info_span!("git_reset_origin_stable"))
        .await?;

    print_stdout_and_stderr(&output);

    info!("Running git checkout -b {git_ref} ...");

    let output = process::Command::new("git")
        .arg("checkout")
        .arg("-b")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .instrument(info_span!("git_checkout_branch"))
        .await?;

    print_stdout_and_stderr(&output);

    info!("Running git checkout {git_ref} ...");

    let output = process::Command::new("git")
        .arg("checkout")
        .arg(git_ref)
        .current_dir(abbs_path)
        .output()
        .instrument(info_span!("git_checkout_branch"))
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
        .instrument(info_span!("git_reset_head"))
        .await?;

    print_stdout_and_stderr(&output);

    if !output.status.success() {
        bail!("Failed to checkout {git_ref}");
    }

    Ok(())
}

pub fn print_stdout_and_stderr(output: &Output) {
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
#[tracing::instrument(skip(pr))]
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
        .draft(true)
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
        ("mipsisa64r6el", vec![String::from("cip-pilot")]),
        ("mipsr6", vec![String::from("cip-pilot")]),
        ("r6", vec![String::from("cip-pilot")]),
        ("linux-kernel", vec![String::from("kernel")]),
        ("new", vec![String::from("new-package")]),
        ("preview", vec![String::from("preview")]),
        ("alpha", vec![String::from("pre-release")]),
        ("beta", vec![String::from("pre-release")]),
        ("rc", vec![String::from("pre-release")]),
        ("pre-release", vec![String::from("pre-release")]),
        ("flight", vec![String::from("flight")]),
        (
            "ftbfs",
            vec![String::from("has-fix"), String::from("ftbfs")],
        ),
        ("rework", vec![String::from("rework")]),
    ];

    for (k, v) in v {
        // assemble regex on the fly
        // e.g. linux-kernel => r"(?i)\blinux-kernel\b"
        // (?i) is enable case insensitive mode
        // \b assert position at a word boundary
        let re = Regex::new(format!(r"(?i)\b{}\b", k).as_str()).unwrap();
        if re.is_match(title).unwrap() {
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
    }

    for i in ["loongson3", "ppc64el", "riscv64"] {
        if archs.contains(&i) {
            s.push_str(&format!("- [ ] {}\n", map[i]));
        }
    }

    s
}

pub fn strip_modifiers(pkg: &str) -> &str {
    match pkg.split_once(":") {
        Some((prefix, _suffix)) => prefix,
        None => pkg,
    }
}

// find autobuild/defines files under `path`
pub fn locate_defines(path: &Path) -> Vec<PathBuf> {
    if path.join("autobuild").exists() {
        vec![path.join("autobuild").join("defines")]
    } else {
        // handle split packages

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
    }
}

/// `packages` should have no groups nor modifiers
#[tracing::instrument(skip(p))]
pub fn get_archs<'a>(p: &'a Path, packages: &'a [String]) -> Vec<&'static str> {
    let mut is_noarch = vec![];
    let mut fail_archs = vec![];

    for_each_abbs(p, |pkg, path| {
        if !packages.contains(&pkg.to_string()) {
            return;
        }

        let defines_list = locate_defines(path);

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
                for a in ALL_ARCH {
                    if !r.is_match(a).unwrap_or(false) && !res.contains(a) {
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

pub fn get_spec(path: &Path, pkgname: &str) -> anyhow::Result<(String, PathBuf)> {
    let mut spec = None;
    for_each_abbs(path, |pkg, p| {
        if pkgname == pkg {
            let p = p.join("spec");
            spec = fs::read_to_string(&p).ok().map(|x| (x, p));
        }
    });

    spec.context(format!("{pkgname} does not exist"))
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

// strip modifiers and expand groups
pub fn resolve_packages(pkgs: &[String], p: &Path) -> anyhow::Result<Vec<String>> {
    let mut req_pkgs = vec![];
    for i in pkgs {
        // strip modifiers: e.g. llvm:+stage2 becomes llvm
        let i = strip_modifiers(i);
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
    Ok(req_pkgs)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EnvironmentRequirement {
    pub min_core: Option<i32>,
    pub min_total_mem: Option<i64>,
    pub min_total_mem_per_core: Option<f32>,
    pub min_disk: Option<i64>,
}

/// `packages` should have no groups nor modifiers
/// Return one ENVREQ for each arch
#[tracing::instrument(skip(p))]
pub fn get_environment_requirement(
    p: &Path,
    packages: &[String],
) -> BTreeMap<&'static str, EnvironmentRequirement> {
    let mut res = BTreeMap::new();

    for_each_abbs(p, |pkg, path| {
        if !packages.contains(&pkg.to_string()) {
            return;
        }

        let spec = path.join("spec");
        let spec = std::fs::read_to_string(spec);

        if let Ok(spec) = spec {
            let spec = read_ab_with_apml(&spec);
            for arch in ALL_ARCH {
                let res_arch: &mut EnvironmentRequirement = res.entry(*arch).or_default();
                if let Some(env_req) = spec
                    .get(&format!("ENVREQ__{}", arch.to_ascii_uppercase()))
                    .or_else(|| spec.get("ENVREQ"))
                {
                    for req in env_req.split(" ") {
                        if let Some((key, value)) = req.split_once("=") {
                            let val = value.parse::<f32>();
                            match (key, val) {
                                ("core", Ok(val)) => {
                                    *res_arch.min_core.get_or_insert(0) =
                                        std::cmp::max(res_arch.min_core.unwrap_or(0), val as i32);
                                }
                                ("total_mem", Ok(val)) => {
                                    // unit: GiB -> B
                                    *res_arch.min_total_mem.get_or_insert(0) = std::cmp::max(
                                        res_arch.min_total_mem.unwrap_or(0),
                                        (val as i64) * 1024 * 1024 * 1024,
                                    );
                                }
                                ("total_mem_per_core", Ok(val)) => {
                                    // unit: GiB
                                    *res_arch.min_total_mem_per_core.get_or_insert(0.0) = f32::max(
                                        res_arch.min_total_mem_per_core.unwrap_or(0.0),
                                        val * 1024.0 * 1024.0 * 1024.0,
                                    );
                                }
                                ("disk", Ok(val)) => {
                                    // unit: GB -> B
                                    *res_arch.min_disk.get_or_insert(0) = std::cmp::max(
                                        res_arch.min_disk.unwrap_or(0),
                                        (val as i64) * 1000 * 1000 * 1000,
                                    );
                                }
                                _ => {
                                    warn!("Unsupported environment requirement: {}", req);
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    res
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

    let title = "linux-kernel-rpi-lts: update to 1234567890";
    let s = auto_add_label(title);
    assert_eq!(s, vec!["upgrade".to_string(), "kernel".to_string()]);

    let title = "update musescore and dropbox";
    let s = auto_add_label(title);
    assert_eq!(s, vec!["upgrade".to_string()]);

    let title = "drOp dropbox";
    let s = auto_add_label(title);
    assert_eq!(s, vec!["drop-package".to_string()]);

    let title = "drop drop drop drop";
    let s = auto_add_label(title);
    assert_eq!(s, vec!["drop-package".to_string()]);

    let title =
        "[PRE-RELEASE]linux-KeRnEl-invalid-version:downgrade?to^0.9~to#fix-0day@CVE-114514-1919810";
    let s = auto_add_label(title);
    assert_eq!(
        s,
        vec![
            "has-fix".to_string(),
            "downgrade".to_string(),
            "security".to_string(),
            "0day".to_string(),
            "kernel".to_string(),
            "pre-release".to_string()
        ]
    );
}
