//! Remote mutation certainty, independent of provider wire types.

/// Effect that may or may not have happened remotely.
#[derive(Debug, Clone)]
pub enum EffectOutcome<T> {
    Definite(T),
    /// Transport-level uncertainty: classify by lookup, never blind-retry.
    Uncertain {
        summary: String,
    },
}

impl<T> EffectOutcome<T> {
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> EffectOutcome<U> {
        match self {
            EffectOutcome::Definite(value) => EffectOutcome::Definite(f(value)),
            EffectOutcome::Uncertain { summary } => EffectOutcome::Uncertain { summary },
        }
    }

    pub fn definite(self) -> Option<T> {
        match self {
            EffectOutcome::Definite(value) => Some(value),
            EffectOutcome::Uncertain { .. } => None,
        }
    }
}
