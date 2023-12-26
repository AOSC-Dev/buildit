use anyhow::anyhow;
use fancy_regex::Regex;
use log::debug;
use std::{collections::HashMap, path::Path};
use walkdir::WalkDir;

use crate::{github::get_repo, ALL_ARCH};

pub fn get_archs<'a>(p: &'a Path, packages: &'a [String]) -> Vec<&'a str> {
    let mut is_noarch = vec![];
    let mut fail_archs = vec![];

    for_each_abbs(p, |pkg, path| {
        if !packages.contains(&pkg.to_string()) {
            return;
        }

        let defines = path.join("autobuild").join("defines");
        let defines = std::fs::read_to_string(defines);

        if let Ok(defines) = defines {
            let defines = read_ab_with_apml(&defines);

            if let Ok(defines) = defines {
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
        // FIXME: loongarch64 is not in the mainline yet and should not be compiled automatically
        // let v = ALL_ARCH.to_vec();
        if fail_archs.iter().any(|x| x.is_none()) {
            ALL_ARCH
                .iter()
                .filter(|x| x != &&"loongarch64")
                .map(|x| x.to_owned())
                .collect()
        } else {
            let mut res = vec![];

            for i in fail_archs {
                let r = i.unwrap();
                for a in ALL_ARCH
                    .iter()
                    .filter(|x| x != &&"loongarch64")
                    .map(|x| x.to_owned())
                {
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

pub fn read_ab_with_apml(file: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut context = HashMap::new();

    // Try to set some ab3 flags to reduce the chance of returning errors
    for i in ["ARCH", "PKGDIR", "SRCDIR"] {
        context.insert(i.to_string(), "".to_string());
    }

    abbs_meta_apml::parse(file, &mut context).map_err(|e| {
        let e: Vec<String> = e.iter().map(|e| e.to_string()).collect();
        anyhow!(e.join("; "))
    })?;

    Ok(context)
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

pub fn find_shorten_id(repo: &Path, git_commit: &str) -> Option<String> {
    let repo = get_repo(repo).ok()?;

    let mut id = None;
    repo.head()
        .ok()?
        .try_into_peeled_id()
        .ok()??
        .ancestors()
        .all()
        .ok()?
        .map_while(Result::ok)
        .for_each(|commit| {
            if git_commit == commit.id.to_string() {
                id =
                    Some(()).and_then(|_| Some(commit.object().ok()?.short_id().ok()?.to_string()));
                return;
            }
        });

    id
}
