use std::fs;
use std::path::{Path, PathBuf};

use clap::ValueHint;
use color_eyre::eyre::{eyre, Result};
use console::style;

use crate::cli::command::Command;
use crate::config::Config;
use crate::dirs;
use crate::file::{make_symlink, remove_all};
use crate::output::Output;

/// Symlinks a plugin into rtx
///
/// This is used for developing a plugin.
#[derive(Debug, clap::Args)]
#[clap(alias = "l", verbatim_doc_comment, after_long_help = AFTER_LONG_HELP)]
pub struct PluginsLink {
    /// The name of the plugin
    /// e.g.: nodejs, ruby
    #[clap(verbatim_doc_comment)]
    name: String,

    /// The local path to the plugin
    /// e.g.: ./rtx-nodejs
    #[clap(value_hint = ValueHint::DirPath, verbatim_doc_comment)]
    path: Option<PathBuf>,

    /// Overwrite existing plugin
    #[clap(long, short = 'f')]
    force: bool,
}

impl Command for PluginsLink {
    fn run(self, _config: Config, _out: &mut Output) -> Result<()> {
        let (name, path) = match self.path {
            Some(path) => (self.name, path),
            None => {
                let path = PathBuf::from(&self.name).canonicalize()?;
                let name = get_name_from_path(&path);
                (name, path)
            }
        };
        let path = path.canonicalize()?;
        let symlink = dirs::PLUGINS.join(&name);
        if symlink.exists() {
            if self.force {
                remove_all(&symlink)?;
            } else {
                return Err(eyre!(
                    "plugin {} already exists, use --force to overwrite",
                    style(&name).cyan().for_stderr()
                ));
            }
        }
        fs::create_dir_all(&*dirs::PLUGINS)?;
        make_symlink(&path, &symlink)?;
        Ok(())
    }
}

fn get_name_from_path(path: &Path) -> String {
    let name = path.file_name().unwrap().to_str().unwrap();
    let name = name.strip_prefix("asdf-").unwrap_or(name);
    let name = name.strip_prefix("rtx-").unwrap_or(name);
    name.to_string()
}

static AFTER_LONG_HELP: &str = color_print::cstr!(
    r#"<bold><underline>Examples:</underline></bold>
  # essentially just `ln -s ./rtx-nodejs ~/.local/share/rtx/plugins/nodejs`
  $ <bold>rtx plugins link nodejs ./rtx-nodejs</bold>

  # infer plugin name as "nodejs"
  $ <bold>rtx plugins link ./rtx-nodejs</bold>
"#
);

#[cfg(test)]
mod tests {
    use crate::{assert_cli, assert_cli_snapshot};

    #[test]
    fn test_plugin_link() {
        assert_cli!("plugin", "link", "tiny-link", "../data/plugins/tiny");
        assert_cli_snapshot!("plugins", "ls");
        assert_cli!("plugin", "uninstall", "tiny-link");
    }
}
