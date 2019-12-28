// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use anyhow::{anyhow, Result};
use slog::warn;
use std::env;
use std::path::{Path, PathBuf};

use super::environment::EnvironmentContext;
use crate::py_packaging::config::{EmbeddedPythonConfig, RawAllocator, RunMode};
use crate::py_packaging::distribution::PythonDistributionLocation;

#[derive(Clone, Debug, PartialEq)]
pub struct BuildConfig {
    pub application_name: String,
    pub build_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InstallLocation {
    Embedded,
    AppRelative { path: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackagingFilterInclude {
    pub files: Vec<String>,
    pub glob_files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackagingWriteLicenseFiles {
    pub path: String,
}

#[derive(Clone, Debug)]
pub enum PythonPackaging {
    FilterInclude(PackagingFilterInclude),
    WriteLicenseFiles(PackagingWriteLicenseFiles),
}

#[derive(Clone, Debug)]
pub struct DistributionTarball {
    pub path_prefix: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DistributionWixInstaller {
    pub msi_upgrade_code_x86: Option<String>,
    pub msi_upgrade_code_amd64: Option<String>,
    pub bundle_upgrade_code: Option<String>,
}

/// Represents a distribution rule.
#[derive(Clone, Debug)]
pub enum Distribution {
    Tarball(DistributionTarball),
    WixInstaller(DistributionWixInstaller),
}

/// Represents a parsed PyOxidizer configuration file.
#[derive(Clone, Debug)]
pub struct Config {
    pub config_path: PathBuf,
    pub build_config: BuildConfig,
    pub embedded_python_config: EmbeddedPythonConfig,
    pub python_distribution: PythonDistributionLocation,
    pub python_packaging: Vec<PythonPackaging>,
    pub run: RunMode,
    pub distributions: Vec<Distribution>,
}

pub fn resolve_install_location(value: &str) -> Result<InstallLocation> {
    if value == "embedded" {
        Ok(InstallLocation::Embedded)
    } else if value.starts_with("app-relative:") {
        let path = value[13..value.len()].to_string();

        Ok(InstallLocation::AppRelative { path })
    } else {
        Err(anyhow!("invalid install_location: {}", value))
    }
}

pub fn default_raw_allocator(target: &str) -> RawAllocator {
    // Jemalloc doesn't work on Windows.
    //
    // We don't use Jemalloc by default in the test environment because it slows down
    // builds of test projects.
    if target == "x86_64-pc-windows-msvc" || cfg!(test) {
        RawAllocator::System
    } else {
        RawAllocator::Jemalloc
    }
}

/// Find a pyoxidizer.toml configuration file by walking directory ancestry.
pub fn find_pyoxidizer_config_file(start_dir: &Path) -> Option<PathBuf> {
    for test_dir in start_dir.ancestors() {
        let candidate = test_dir.to_path_buf().join("pyoxidizer.bzl");

        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

/// Find a PyOxidizer configuration file from walking the filesystem or an
/// environment variable override.
pub fn find_pyoxidizer_config_file_env(logger: &slog::Logger, start_dir: &Path) -> Option<PathBuf> {
    match env::var("PYOXIDIZER_CONFIG") {
        Ok(config_env) => {
            warn!(
                logger,
                "using PyOxidizer config file from PYOXIDIZER_CONFIG: {}", config_env
            );
            Some(PathBuf::from(config_env))
        }
        Err(_) => find_pyoxidizer_config_file(start_dir),
    }
}

pub fn eval_starlark_config_file(
    logger: &slog::Logger,
    path: &Path,
    build_target: &str,
) -> Result<Config> {
    let context = EnvironmentContext::new(logger, path, build_target)?;

    let res = crate::starlark::eval::evaluate_file(logger, path, &context)
        .or_else(|d| Err(anyhow!(d.message)))?;

    let config = res
        .env
        .get("CONFIG")
        .or_else(|_| Err(anyhow!("CONFIG does not exist")))?;

    if config.get_type() != "Config" {
        return Err(anyhow!(
            "CONFIG must be type Config; got type {}",
            config.get_type()
        ));
    }

    Ok(config.downcast_apply(|x: &crate::starlark::config::Config| -> Config { x.config.clone() }))
}
