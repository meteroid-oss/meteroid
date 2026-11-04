use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `<commit>[.+] (<branch>)`, the format build-info produced.
fn git_info() -> Option<String> {
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])?;
    println!("cargo:rerun-if-changed={git_dir}/HEAD");

    let commit = git(&["rev-parse", "HEAD"])?;
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    let branch = git(&["symbolic-ref", "--short", "-q", "HEAD"]);

    if let Some(branch) = &branch {
        let common_dir = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .unwrap_or_else(|| git_dir.clone());
        let common_dir = Path::new(&common_dir);
        let reference = common_dir.join("refs/heads").join(branch);
        let watched = if reference.is_file() {
            reference
        } else {
            common_dir.join("packed-refs")
        };
        println!("cargo:rerun-if-changed={}", watched.display());
    }

    let mut info = commit;
    if dirty {
        info.push_str(".+");
    }
    if let Some(branch) = branch {
        info.push_str(&format!(" ({branch})"));
    }
    Some(info)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GIT_SHA");

    // Docker builds have no .git (see .dockerignore): they pass the commit instead.
    let info = std::env::var("GIT_SHA")
        .ok()
        .filter(|sha| !sha.is_empty())
        .or_else(git_info)
        .unwrap_or_else(|| "None".to_owned());

    println!("cargo:rustc-env=COMMON_BUILD_INFO_GIT={info}");
    println!(
        "cargo:rustc-env=COMMON_BUILD_INFO_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "UNKNOWN".to_owned())
    );
}
