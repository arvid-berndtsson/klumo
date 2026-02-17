use super::{Cli, Commands, bundle_command, eval_command, fmt_command, install_dependencies};
use super::{lint_command, repl_command, run_command, test_command};
use anyhow::Result;

pub(crate) fn execute(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Commands::Run {
            file,
            config,
            lang,
            print_js,
            no_cache,
            force_llm,
            self_heal,
            max_heal_attempts,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => {
            if let Some(path) = file {
                run_command(
                    path,
                    config,
                    lang,
                    print_js,
                    no_cache,
                    force_llm,
                    self_heal,
                    max_heal_attempts,
                    no_progress,
                    verbose,
                    provider,
                    ollama_url,
                    model,
                )
            } else {
                repl_command(
                    config,
                    lang,
                    print_js,
                    no_cache,
                    no_progress,
                    verbose,
                    provider,
                    ollama_url,
                    model,
                )
            }
        }
        Some(Commands::Eval { code }) => eval_command(code),
        Some(Commands::Bundle {
            file,
            output,
            config,
            lang,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => bundle_command(
            file,
            output,
            config,
            lang,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        ),
        Some(Commands::Install { config, dry_run }) => install_dependencies(config, dry_run),
        Some(Commands::Lint { fix, paths }) => lint_command(paths, fix),
        Some(Commands::Fmt { check, paths }) => fmt_command(paths, check),
        Some(Commands::Test { paths }) => test_command(paths),
        Some(Commands::Repl {
            config,
            lang,
            print_js,
            no_cache,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => repl_command(
            config,
            lang,
            print_js,
            no_cache,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        ),
        None => repl_command(None, None, false, false, false, false, None, None, None),
    }
}
