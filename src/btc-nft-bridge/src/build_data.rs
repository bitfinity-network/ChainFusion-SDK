use candid::CandidType;
use serde::{Deserialize, Serialize};

// E.g.: x86_64-unknown-linux-gnu
const CARGO_TARGET_TRIPLE: &str = env!("VERGEN_CARGO_TARGET_TRIPLE");
// E.g.: default
const CARGO_FEATURES: &str = env!("VERGEN_CARGO_FEATURES");
// E.g.: evm
const PKG_NAME: &str = env!("CARGO_PKG_NAME");
// E.g.: 0.1.0
const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
// E.g.: 1.64.0
const RUSTC_SEMVER: &str = env!("VERGEN_RUSTC_SEMVER");
// E.g.: 2022-12-23T15:29:20.000000000Z
const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
// E.g.: true/false
const CARGO_DEBUG: &str = env!("VERGEN_CARGO_DEBUG");
// E.g.: main
const GIT_BRANCH: &str = env!("VERGEN_GIT_BRANCH");
// E.g.: acf6c5744b1f4f29c5960a25f4fb4056e2ceedc3
const GIT_SHA: &str = env!("VERGEN_GIT_SHA");
// E.g.: 2022-12-23T15:29:20.000000000Z
const GIT_COMMIT_TIMESTAMP: &str = env!("VERGEN_GIT_COMMIT_TIMESTAMP");

/// Contains the build data.
#[derive(Debug, Clone, Serialize, Deserialize, CandidType)]
pub struct BuildData {
    pub cargo_target_triple: String,
    pub cargo_features: String,
    pub pkg_name: String,
    pub pkg_version: String,
    pub rustc_semver: String,
    pub build_timestamp: String,
    pub cargo_debug: String,
    pub git_branch: String,
    pub git_sha: String,
    pub git_commit_timestamp: String,
}

/// Returns the build data.
pub fn canister_build_data() -> BuildData {
    BuildData {
        cargo_target_triple: CARGO_TARGET_TRIPLE.to_string(),
        cargo_features: CARGO_FEATURES.to_string(),
        pkg_name: PKG_NAME.to_string(),
        pkg_version: PKG_VERSION.to_string(),
        rustc_semver: RUSTC_SEMVER.to_string(),
        build_timestamp: BUILD_TIMESTAMP.to_string(),
        cargo_debug: CARGO_DEBUG.to_string(),
        git_branch: GIT_BRANCH.to_string(),
        git_sha: GIT_SHA.to_string(),
        git_commit_timestamp: GIT_COMMIT_TIMESTAMP.to_string(),
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn should_create_build_data() {
        let build_data = canister_build_data();

        assert_eq!(build_data.pkg_name, "brc20-bridge");

        assert_eq!(build_data.cargo_target_triple, CARGO_TARGET_TRIPLE);
        assert_eq!(build_data.cargo_features, CARGO_FEATURES);
        assert_eq!(build_data.pkg_name, PKG_NAME);
        assert_eq!(build_data.pkg_version, PKG_VERSION);
        assert_eq!(build_data.rustc_semver, RUSTC_SEMVER);
        assert_eq!(build_data.build_timestamp, BUILD_TIMESTAMP);
        assert_eq!(build_data.cargo_debug, CARGO_DEBUG);
        assert_eq!(build_data.git_branch, GIT_BRANCH);
        assert_eq!(build_data.git_sha, GIT_SHA);
        assert_eq!(build_data.git_commit_timestamp, GIT_COMMIT_TIMESTAMP);

        assert!(!build_data.cargo_target_triple.is_empty());
        // assert!(!build_data.cargo_features.is_empty()); Cargo features can be empty.
        assert!(!build_data.pkg_name.is_empty());
        assert!(!build_data.pkg_version.is_empty());
        assert!(!build_data.rustc_semver.is_empty());
        assert!(!build_data.build_timestamp.is_empty());
        assert!(!build_data.cargo_debug.is_empty());
        assert!(!build_data.git_branch.is_empty());
        assert!(!build_data.git_sha.is_empty());
        assert!(!build_data.git_commit_timestamp.is_empty());

        println!("build data: {:?}", build_data)
    }
}
