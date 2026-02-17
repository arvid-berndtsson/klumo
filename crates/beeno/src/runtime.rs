use anyhow::{Result, anyhow};
use boa_engine::{Context, Source};

pub struct BeenoRuntime {
    ctx: Context,
}

impl BeenoRuntime {
    pub fn new() -> Self {
        Self {
            ctx: Context::default(),
        }
    }

    pub fn eval_script(&mut self, source: &str, source_name: &str) -> Result<Option<String>> {
        let result = self
            .ctx
            .eval(Source::from_bytes(source))
            .map_err(|err| anyhow!("failed evaluating {source_name}: {err}"))?;

        if result.is_undefined() {
            return Ok(None);
        }

        let rendered = result
            .to_string(&mut self.ctx)
            .map_err(|err| anyhow!("failed converting JS value to string: {err}"))?;
        Ok(Some(rendered.to_std_string_escaped()))
    }
}
