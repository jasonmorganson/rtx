use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::exit;

use color_eyre::eyre::{eyre, Result};
use indoc::formatdoc;
use rayon::prelude::*;

use crate::cli::command::Command;
use crate::cli::exec::Exec;
use crate::config::Config;
use crate::env;
use crate::fake_asdf;
use crate::file::{create_dir_all, remove_all};
use crate::lock_file::LockFile;
use crate::output::Output;
use crate::plugins::{Plugin, Plugins};
use crate::runtimes::RuntimeVersion;
use crate::toolset::{Toolset, ToolsetBuilder};
use crate::{dirs, file};

// executes as if it was a shim if the command is not "rtx", e.g.: "node"
#[allow(dead_code)]
pub fn handle_shim(mut config: Config, args: &[String], out: &mut Output) -> Result<Config> {
    let (_, bin_name) = args[0].rsplit_once('/').unwrap_or(("", &args[0]));
    if bin_name == "rtx" || !config.settings.experimental {
        return Ok(config);
    }
    let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
    args[0] = which_shim(&mut config, bin_name)?.into();
    let exec = Exec {
        runtime: vec![],
        c: None,
        command: Some(args),
        cd: None,
    };
    exec.run(config, out)?;
    exit(0);
}

fn which_shim(config: &mut Config, bin_name: &str) -> Result<PathBuf> {
    if let Ok(shims_dir) = config.get_shims_dir() {
        let shim = shims_dir.join(bin_name);
        if shim.exists() {
            let ts = ToolsetBuilder::new().build(config)?;
            if let Some(rtv) = ts.which(config, bin_name) {
                if let Some(bin) = rtv.which(&config.settings, bin_name)? {
                    return Ok(bin);
                }
            }
            // fallback for "system"
            for path in &*env::PATH {
                if fs::canonicalize(path).unwrap_or_default()
                    == fs::canonicalize(&shims_dir).unwrap_or_default()
                {
                    continue;
                }
                let bin = path.join(bin_name);
                if bin.exists() {
                    return Ok(bin);
                }
            }
            let rtvs = ts.list_rtvs_with_bin(config, bin_name)?;
            err_no_version_set(bin_name, rtvs)?;
        }
    }
    Err(eyre!("{} is not a valid shim", bin_name))
}

pub fn reshim(config: &mut Config, ts: &Toolset) -> Result<()> {
    if !config.settings.experimental || config.settings.shims_dir.is_none() {
        return Ok(());
    }
    let shims_dir = config.get_shims_dir()?;
    let _lock = LockFile::new(&shims_dir)
        .with_callback(|l| {
            trace!("reshim callback {}", l.display());
        })
        .lock();

    // remove old shims
    let _ = remove_all(&shims_dir);
    create_dir_all(&shims_dir)?;
    let rtx_bin = config.rtx_bin().unwrap_or(env::RTX_EXE.clone());

    let paths: Vec<PathBuf> = ts
        .list_installed_versions(config)?
        .into_par_iter()
        .flat_map(|rtv| match rtv.list_bin_paths(&config.settings) {
            Ok(paths) => paths.clone(),
            Err(e) => {
                warn!("Error listing bin paths for {}: {:#}", rtv, e);
                Vec::new()
            }
        })
        .collect();

    for path in paths {
        if !path.exists() {
            continue;
        }
        for bin in path.read_dir()? {
            let bin = bin?;
            if !bin.file_type()?.is_file() && !bin.file_type()?.is_symlink() {
                continue;
            }
            let bin_name = bin.file_name().into_string().unwrap();
            let symlink_path = shims_dir.join(bin_name);
            file::make_symlink(&rtx_bin, &symlink_path).map_err(|err| {
                eyre!(
                    "Failed to create symlink from {} to {}: {}",
                    rtx_bin.display(),
                    symlink_path.display(),
                    err
                )
            })?;
        }
    }
    for plugin in config.plugins.values() {
        match plugin.as_ref() {
            Plugins::External(plugin) => match plugin.plugin_path.join("shims").read_dir() {
                Ok(files) => {
                    for bin in files {
                        let bin = bin?;
                        let bin_name = bin.file_name().into_string().unwrap();
                        let symlink_path = shims_dir.join(bin_name);
                        make_shim(&bin.path(), &symlink_path)?;
                    }
                }
                Err(_) => {
                    continue;
                }
            },
        }
    }

    Ok(())
}

fn make_shim(target: &Path, shim: &Path) -> Result<()> {
    if shim.exists() {
        fs::remove_file(shim)?;
    }
    fs::write(
        shim,
        formatdoc! {r#"
        #!/bin/sh
        export ASDF_DATA_DIR={data_dir}
        export PATH="{fake_asdf_dir}:$PATH"
        rtx x -- {target} "$@"
        "#,
        data_dir = dirs::ROOT.display(),
        fake_asdf_dir = fake_asdf::setup()?.display(),
        target = target.display()},
    )?;
    let mut perms = shim.metadata()?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(shim, perms)?;
    trace!(
        "shim created from {} to {}",
        target.display(),
        shim.display()
    );
    Ok(())
}

fn err_no_version_set(bin_name: &str, rtvs: Vec<RuntimeVersion>) -> Result<()> {
    if rtvs.is_empty() {
        return Ok(());
    }
    let mut msg = format!("No version is set for shim: {}\n", bin_name);
    msg.push_str("Set a global default version with one of the following:\n");
    for rtv in rtvs {
        msg.push_str(&format!(
            "rtx global {}@{}\n",
            rtv.plugin.name(),
            rtv.version
        ));
    }
    Err(eyre!(msg.trim().to_string()))
}
