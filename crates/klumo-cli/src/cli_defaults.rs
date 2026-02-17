use anyhow::{Context, Result};
use klumo_config::{FileConfig, load_file_config};
use std::ffi::OsString;

const PREDEFINED_COMMANDS: &[&str] = &[
    "run", "bundle", "install", "i", "lint", "fmt", "test", "eval", "repl",
];

pub(crate) fn normalize_cli_args<I>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = OsString>,
{
    let mut normalized: Vec<OsString> = args.into_iter().collect();
    if normalized.len() < 2 {
        return normalized;
    }

    let first = normalized[1].to_string_lossy();
    let is_known_subcommand = PREDEFINED_COMMANDS.contains(&first.as_ref());
    let is_flag = first.starts_with('-');

    if !is_known_subcommand && !is_flag {
        normalized.insert(1, OsString::from("run"));
    }

    normalized
}

pub(crate) fn predefined_script_collisions(cfg: &FileConfig) -> Vec<String> {
    let Some(scripts) = cfg.scripts.as_ref() else {
        return Vec::new();
    };

    scripts
        .keys()
        .filter(|name| PREDEFINED_COMMANDS.contains(&name.as_str()))
        .cloned()
        .collect()
}

pub(crate) fn warn_predefined_script_collisions() -> Result<()> {
    let cwd = std::env::current_dir().context("failed resolving current directory")?;
    let cfg = load_file_config(None, &cwd)?;
    let Some(cfg) = cfg else {
        return Ok(());
    };

    for name in predefined_script_collisions(&cfg) {
        eprintln!(
            "[klumo] warning: scripts.{name} in {}/klumo.json conflicts with a built-in command and may be ignored",
            cwd.display()
        );
    }

    Ok(())
}
