use anyhow::{Result, anyhow};
use beeno_engine::{EvalOutput, JsEngine};

/// Placeholder V8 backend entrypoint.
///
/// This crate establishes the engine boundary and dependency wiring so we can
/// swap Boa for a V8 implementation without changing CLI UX.
#[derive(Debug)]
pub struct V8Engine;

impl V8Engine {
    pub fn new() -> Result<Self> {
        Err(anyhow!(
            "V8 backend is scaffolded but not implemented yet. Use BEENO_ENGINE=boa for now."
        ))
    }
}

impl JsEngine for V8Engine {
    fn eval_script(&mut self, _source: &str, _source_name: &str) -> Result<EvalOutput> {
        Err(anyhow!(
            "V8 backend is scaffolded but not implemented yet. Use BEENO_ENGINE=boa for now."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::V8Engine;

    #[test]
    fn v8_engine_reports_unavailable() {
        let err = V8Engine::new().expect_err("v8 is not implemented yet");
        assert!(err.to_string().contains("not implemented"));
    }
}
