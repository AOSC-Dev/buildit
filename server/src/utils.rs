use anyhow::anyhow;
use fancy_regex::Regex;
use log::debug;
use std::{collections::HashMap, path::Path};
use walkdir::WalkDir;

pub fn read_ab_with_apml(file: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut context = HashMap::new();

    // Try to set some ab3 flags to reduce the chance of returning errors
    for i in ["ARCH", "PKGDIR", "SRCDIR"] {
        context.insert(i.to_string(), "".to_string());
    }

    abbs_meta_apml::parse(file, &mut context).map_err(|e| {
        let e: Vec<String> = e.iter().map(|e| e.to_string()).collect();
        anyhow!(e.join(": "))
    })?;

    Ok(context)
}

// pub fn is_noarch_and_fail_arch(pkgs: &[String], path: &Path) -> anyhow::Result<bool> {
//     let mut res = None;
//     // for i in WalkDir::new(path)
//     //     .max_depth(2)
//     //     .min_depth(2)
//     //     .into_iter()
//     //     .flatten()
//     // {
//     //     if i.path().is_file() {
//     //         continue;
//     //     }

//     //     let pkg = i.file_name().to_str();

//     //     if pkg.is_none() {
//     //         debug!("Failed to convert str: {}", i.path().display());
//     //         continue;
//     //     }

//     //     let pkg = pkg.unwrap();
//     //     if pkgs.contains(&pkg.to_string()) {
//     //         let defines = i.path().join("autobuild").join("defines");
//     //         let defines = std::fs::read_to_string(defines);

//     //         if let Ok(defines) = defines {
//     //             let map = read_ab_with_apml(&defines)?;

//     //             let b = map
//     //                 .get("ABHOST")
//     //                 .map(|x| x.to_ascii_lowercase() == "noarch")
//     //                 .unwrap_or(false);

//     //             res = Some(b);

//     //             if !b {
//     //                 return Ok(false);
//     //             }
//     //         }
//     //     }
//     // }

//     for_each_tree(path, move |pkg| {
//         if pkgs.
//     });

//     Ok(res.is_some())
// }

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
