use std::fmt::Formatter;

#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub name: String,
    pub version: String,
    // debug | release
    pub profile: String,
    pub target_family: String,
    pub target_os: String,
    pub target_arch: String,
    pub git_info: String,
}

impl std::fmt::Display for BuildInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} [{}] ({}) {}-{}-{}",
            self.name,
            self.version,
            self.profile,
            self.git_info,
            self.target_family,
            self.target_os,
            self.target_arch,
        )
    }
}

impl BuildInfo {
    fn build(name: &str) -> Self {
        Self {
            name: name.to_string(),
            version: build_info::format!("v{}", $.crate_info.version).to_string(),
            profile: build_info::format!("{}", $.profile).to_string(),
            target_family: build_info::format!("{}", $.target.family).to_string(),
            target_os: build_info::format!("{}", $.target.os).to_string(),
            target_arch: build_info::format!("{}", $.target.cpu.arch).to_string(),
            git_info: build_info::format!("{}", $.version_control).to_string(),
        }
    }

    pub fn get() -> &'static Self {
        INSTANCE
            .get()
            .expect("BuildInfo value is not set on binary start")
    }

    pub fn set(name: &str) -> &'static Self {
        match INSTANCE.get() {
            None => {
                let build_info = Self::build(name);
                INSTANCE
                    .set(build_info)
                    .expect("Failed to set BuildInfo value");
                BuildInfo::get()
            }
            Some(v) => {
                panic!("BuildInfo value is already set {:?}", v);
            }
        }
    }
}

static INSTANCE: std::sync::OnceLock<BuildInfo> = std::sync::OnceLock::new();
