use anyhow::{Context, Result, anyhow};
use klumo_config::{FileConfig, load_file_config};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

fn resolve_install_config(config: Option<&Path>) -> Result<FileConfig> {
    let cwd = std::env::current_dir().context("failed resolving current directory")?;
    load_file_config(config, &cwd)?
        .ok_or_else(|| anyhow!("klumo.json not found in {}", cwd.display()))
}

fn resolve_project_script(name: &str) -> Result<Option<String>> {
    let cwd = std::env::current_dir().context("failed resolving current directory")?;
    let cfg = load_file_config(None, &cwd)?;
    Ok(cfg
        .and_then(|file| file.scripts)
        .and_then(|scripts| scripts.get(name).cloned()))
}

fn command_available(program: &str) -> bool {
    Command::new(program).arg("--version").status().is_ok()
}

pub(crate) fn should_prefer_deno_tooling(cwd: &Path) -> bool {
    let has_deno_config = cwd.join("deno.json").exists() || cwd.join("deno.jsonc").exists();
    if has_deno_config {
        return true;
    }

    let has_cargo_manifest = cwd.join("Cargo.toml").exists();
    let has_js_ts_manifest = cwd.join("package.json").exists()
        || cwd.join("tsconfig.json").exists()
        || cwd.join("jsconfig.json").exists();

    has_js_ts_manifest && !has_cargo_manifest
}

fn use_deno_default() -> Result<bool> {
    if !command_available("deno") {
        return Ok(false);
    }
    let cwd = std::env::current_dir().context("failed resolving current directory")?;
    Ok(should_prefer_deno_tooling(&cwd))
}

pub(crate) fn dependency_to_jsr_spec(name: &str, version: &str) -> String {
    let dep = name.trim();
    let version = version.trim();

    let base = if dep.starts_with("jsr:") {
        dep.to_string()
    } else {
        format!("jsr:{dep}")
    };

    if version.is_empty() {
        return base;
    }

    format!("{base}@{version}")
}

pub(crate) fn resolve_run_script_target(
    config: Option<&Path>,
    target: &Path,
) -> Result<Option<String>> {
    let script_name = match target.to_str() {
        Some(value) => value,
        None => return Ok(None),
    };
    let cwd = std::env::current_dir().context("failed resolving current directory")?;
    let file_cfg = load_file_config(config, &cwd)?;
    Ok(file_cfg
        .and_then(|cfg| cfg.scripts)
        .and_then(|scripts| scripts.get(script_name).cloned()))
}

pub(crate) fn install_dependencies(config: Option<PathBuf>, dry_run: bool) -> Result<()> {
    let cfg = resolve_install_config(config.as_deref())?;

    if let Some(install_script) = cfg.scripts.and_then(|scripts| scripts.get("install").cloned()) {
        if dry_run {
            println!("dry-run: would run install script: {install_script}");
            return Ok(());
        }
        run_script_command("install", &install_script)?;
        return Ok(());
    }

    let deps = cfg.dependencies.unwrap_or_default();
    if deps.is_empty() {
        println!("No dependencies found in klumo.json.");
        return Ok(());
    }

    let deno_check = Command::new("deno")
        .arg("--version")
        .status()
        .context("failed to execute 'deno --version'")?;
    if !deno_check.success() {
        return Err(anyhow!(
            "deno is required to install dependencies. Install deno or add scripts.install in klumo.json."
        ));
    }

    for (name, version) in deps {
        let spec = dependency_to_jsr_spec(&name, &version);
        if dry_run {
            println!("dry-run: deno cache {spec}");
            continue;
        }

        println!("Installing {spec}");
        let status = Command::new("deno")
            .args(["cache", spec.as_str()])
            .status()
            .with_context(|| format!("failed running 'deno cache {spec}'"))?;
        if !status.success() {
            return Err(anyhow!("dependency install failed for {spec}"));
        }
    }

    Ok(())
}

fn run_command_with_status(program: &str, args: &[OsString], display: &str) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed running {program} {display}"))?;

    if status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "{program} {} failed with exit status {}",
        display,
        status
    ))
}

pub(crate) fn lint_command(paths: Vec<PathBuf>, fix: bool) -> Result<()> {
    if let Some(script) = resolve_project_script("lint")? {
        return run_script_command("lint", &script);
    }

    if use_deno_default()? {
        let mut args = vec![OsString::from("lint")];
        if fix {
            args.push(OsString::from("--fix"));
        }
        for path in paths {
            args.push(path.into_os_string());
        }
        return run_command_with_status("deno", &args, "lint");
    }

    let mut args = vec![
        OsString::from("clippy"),
        OsString::from("--all-targets"),
        OsString::from("--all-features"),
    ];
    if fix {
        args.push(OsString::from("--fix"));
        args.push(OsString::from("--allow-dirty"));
        args.push(OsString::from("--allow-staged"));
    }
    for path in paths {
        args.push(path.into_os_string());
    }
    run_command_with_status("cargo", &args, "clippy")
}

pub(crate) fn fmt_command(paths: Vec<PathBuf>, check: bool) -> Result<()> {
    if let Some(script) = resolve_project_script("fmt")? {
        return run_script_command("fmt", &script);
    }

    if use_deno_default()? {
        let mut args = vec![OsString::from("fmt")];
        if check {
            args.push(OsString::from("--check"));
        }
        for path in paths {
            args.push(path.into_os_string());
        }
        return run_command_with_status("deno", &args, "fmt");
    }

    if !paths.is_empty() {
        eprintln!(
            "[klumo] warning: 'klumo fmt <paths...>' is not supported for cargo fmt; formatting full workspace instead"
        );
    }

    let mut args = vec![OsString::from("fmt"), OsString::from("--all")];
    if check {
        args.push(OsString::from("--check"));
    }
    run_command_with_status("cargo", &args, "fmt")
}

pub(crate) fn test_command(args: Vec<OsString>) -> Result<()> {
    let mut deno_args = vec![OsString::from("test")];
    deno_args.extend(args);
    run_command_with_status("deno", &deno_args, "test")
}

pub(crate) fn run_script_command(script_name: &str, command_line: &str) -> Result<()> {
    #[cfg(windows)]
    let status = Command::new("cmd")
        .args(["/C", command_line])
        .status()
        .with_context(|| format!("failed running script '{script_name}'"))?;

    #[cfg(not(windows))]
    let status = Command::new("sh")
        .args(["-lc", command_line])
        .status()
        .with_context(|| format!("failed running script '{script_name}'"))?;

    if status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "script '{}' failed with exit status {}",
        script_name,
        status
    ))
}
