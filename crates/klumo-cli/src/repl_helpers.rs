use anyhow::{Context, Result, anyhow};
use klumo_engine::JsEngine;
use std::collections::{HashSet, VecDeque};

pub(crate) fn sanitize_repl_javascript(input: &str) -> String {
    let mut output = String::new();
    for line in input.lines() {
        let trimmed = line.trim_start();

        if let Some(rest) = trimmed.strip_prefix("export default ") {
            output.push_str(rest);
            output.push('\n');
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("export ") {
            output.push_str(rest);
            output.push('\n');
            continue;
        }

        if trimmed.starts_with("import ") {
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    output.trim_end().to_string()
}

pub(crate) fn read_global_names(engine: &mut dyn JsEngine) -> Result<HashSet<String>> {
    let out = engine
        .eval_script(
            "JSON.stringify(Object.getOwnPropertyNames(globalThis))",
            "<repl-scope>",
        )
        .context("failed reading REPL global scope")?;
    let raw = out
        .value
        .ok_or_else(|| anyhow!("scope probe returned empty result"))?;
    let names: Vec<String> =
        serde_json::from_str(&raw).context("failed parsing REPL global scope JSON")?;
    Ok(names.into_iter().collect())
}

fn scope_context_text(bindings: &HashSet<String>) -> Option<String> {
    if bindings.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = bindings.iter().map(String::as_str).collect();
    names.sort_unstable();
    Some(format!(
        "Bindings currently defined in this REPL session: {}. Avoid redeclaring them with const/let/class.",
        names.join(", ")
    ))
}

fn joined_recent_entries(history: &VecDeque<String>) -> Option<String> {
    if history.is_empty() {
        return None;
    }

    let joined = history
        .iter()
        .enumerate()
        .map(|(idx, item)| format!("{}. {}", idx + 1, item))
        .collect::<Vec<_>>()
        .join("\n");
    Some(joined)
}

pub(crate) fn build_repl_scope_context(
    bindings: &HashSet<String>,
    statement_history: &VecDeque<String>,
    js_history: &VecDeque<String>,
    web_server_context: Option<&str>,
) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(bindings_text) = scope_context_text(bindings) {
        sections.push(bindings_text);
    }
    if let Some(statements) = joined_recent_entries(statement_history) {
        sections.push(format!(
            "Previously run REPL statements (oldest to newest):\n{}",
            statements
        ));
    }
    if let Some(js_snippets) = joined_recent_entries(js_history) {
        sections.push(format!(
            "Previously generated JavaScript snippets (oldest to newest):\n{}",
            js_snippets
        ));
    }
    if let Some(web_context) = web_server_context {
        sections.push(web_context.to_string());
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

pub(crate) fn push_bounded(history: &mut VecDeque<String>, item: String, cap: usize) {
    history.push_back(item);
    while history.len() > cap {
        history.pop_front();
    }
}

pub(crate) fn build_repl_self_heal_request(
    user_prompt: &str,
    generated_js: Option<&str>,
    error_text: &str,
    attempt: usize,
) -> String {
    let stage = if generated_js.is_some() {
        "runtime execution after JS generation"
    } else {
        "translation/compile before execution"
    };
    let first_error_line = error_text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("unknown error");
    let probable_cause = if error_text.contains("ReferenceError") {
        "Undefined variable or symbol usage."
    } else if error_text.contains("TypeError") {
        "Invalid operation on value type (often null/undefined access)."
    } else if error_text.contains("SyntaxError") {
        "Generated JS contains invalid syntax."
    } else if error_text.contains("failed evaluating") {
        "Runtime engine rejected or failed while evaluating the generated script."
    } else if error_text.contains("OPENAI_API_KEY") || error_text.contains("llm unavailable") {
        "Provider/config issue prevented translation."
    } else {
        "General execution/translation failure; inspect exact error text."
    };
    let js_section = generated_js.map_or_else(
        || "FAILED JAVASCRIPT:\n<none produced before failure>\n\n".to_string(),
        |js| format!("FAILED JAVASCRIPT:\n{}\n\n", js),
    );
    format!(
        "You are repairing a failed Klumo REPL attempt.\n\
Goal: produce JavaScript that runs successfully in the current REPL session.\n\
Return ONLY runnable JavaScript script statements. No markdown, no prose, no import/export.\n\
Keep user intent and behavior as close as possible.\n\n\
FAILURE REPORT\n\
- Attempt number: {}\n\
- Failure stage: {}\n\
- Error summary: {}\n\
- Probable cause: {}\n\n\
USER PROMPT:\n{}\n\n\
{}\
FULL ERROR OUTPUT:\n{}\n\n\
REPAIR REQUIREMENTS\n\
1. Fix the direct cause of the error.\n\
2. Keep side effects minimal and preserve existing REPL bindings when possible.\n\
3. Output only executable script statements.\n",
        attempt + 1,
        stage,
        first_error_line,
        probable_cause,
        user_prompt,
        js_section,
        error_text
    )
}

pub(crate) fn repl_self_heal_limit() -> Option<usize> {
    let raw = match std::env::var("KLUMO_REPL_SELF_HEAL_MAX_ATTEMPTS") {
        Ok(value) => value,
        Err(_) => return None,
    };
    match raw.trim().parse::<usize>() {
        Ok(0) => None,
        Ok(limit) => Some(limit),
        Err(_) => None,
    }
}

pub(crate) fn can_continue_self_heal(attempt: usize, limit: Option<usize>) -> bool {
    match limit {
        Some(max) => attempt < max,
        None => true,
    }
}

pub(crate) fn is_non_recoverable_self_heal_error(error_text: &str) -> bool {
    error_text.contains("OPENAI_API_KEY")
        || error_text.contains("llm unavailable")
        || error_text.contains("unknown provider")
}
