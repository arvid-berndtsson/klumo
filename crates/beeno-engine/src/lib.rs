use anyhow::{Result, anyhow};
use boa_engine::{Context, Source};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnostic {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalOutput {
    pub value: Option<String>,
    pub diagnostics: Vec<RuntimeDiagnostic>,
}

pub trait JsEngine {
    fn eval_script(&mut self, source: &str, source_name: &str) -> Result<EvalOutput>;
}

pub struct BoaEngine {
    ctx: Context,
}

impl BoaEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            ctx: Context::default(),
        };
        engine.install_console_shim();
        engine
    }

    fn install_console_shim(&mut self) {
        // Provide minimal console support for translated code.
        let _ = self.ctx.eval(Source::from_bytes(
            r#"
globalThis.__beeno_console_logs = [];
globalThis.console = globalThis.console || {};
globalThis.console.log = (...args) => {
  globalThis.__beeno_console_logs.push(args.map((v) => String(v)).join(" "));
};
globalThis.console.error = (...args) => {
  globalThis.__beeno_console_logs.push(args.map((v) => String(v)).join(" "));
};
globalThis.__beeno_flush_console = () => {
  const out = globalThis.__beeno_console_logs.join("\n");
  globalThis.__beeno_console_logs = [];
  return out;
};
"#,
        ));
    }

    fn flush_console_logs(&mut self) {
        let flushed = self
            .ctx
            .eval(Source::from_bytes("globalThis.__beeno_flush_console?.() ?? ''"));
        let Ok(value) = flushed else {
            return;
        };
        let Ok(text) = value.to_string(&mut self.ctx) else {
            return;
        };
        let rendered = text.to_std_string_escaped();
        if !rendered.is_empty() {
            println!("{rendered}");
        }
    }
}

impl Default for BoaEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl JsEngine for BoaEngine {
    fn eval_script(&mut self, source: &str, source_name: &str) -> Result<EvalOutput> {
        let result = self
            .ctx
            .eval(Source::from_bytes(source))
            .map_err(|err| anyhow!("failed evaluating {source_name}: {err}"))?;

        self.flush_console_logs();

        if result.is_undefined() {
            return Ok(EvalOutput {
                value: None,
                diagnostics: Vec::new(),
            });
        }

        let rendered = result
            .to_string(&mut self.ctx)
            .map_err(|err| anyhow!("failed converting JS value to string: {err}"))?
            .to_std_string_escaped();

        Ok(EvalOutput {
            value: Some(rendered),
            diagnostics: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{BoaEngine, JsEngine};

    #[test]
    fn evaluates_expression() {
        let mut engine = BoaEngine::new();
        let output = engine.eval_script("1 + 2", "<test>").expect("eval should pass");
        assert_eq!(output.value.as_deref(), Some("3"));
    }

    #[test]
    fn suppresses_undefined() {
        let mut engine = BoaEngine::new();
        let output = engine
            .eval_script("const a = 1;", "<test>")
            .expect("eval should pass");
        assert_eq!(output.value, None);
    }

    #[test]
    fn maps_runtime_errors() {
        let mut engine = BoaEngine::new();
        let err = engine
            .eval_script("throw new Error('boom')", "sample.js")
            .expect_err("expected eval error");
        assert!(err.to_string().contains("failed evaluating sample.js"));
    }

    #[test]
    fn console_log_does_not_throw() {
        let mut engine = BoaEngine::new();
        let output = engine
            .eval_script("console.log('hello'); 7", "<test>")
            .expect("eval should pass");
        assert_eq!(output.value.as_deref(), Some("7"));
    }

    #[test]
    fn console_logs_flush_on_undefined_result() {
        let mut engine = BoaEngine::new();
        let output = engine
            .eval_script("console.log('hello from undefined');", "<test>")
            .expect("eval should pass");
        assert_eq!(output.value, None);

        let length = engine
            .eval_script("globalThis.__beeno_console_logs.length", "<test>")
            .expect("eval should pass");
        assert_eq!(length.value.as_deref(), Some("0"));
    }
}
