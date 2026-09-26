use super::{RunnerOperation, RunnerRegistrations};
use shaula_core::{diagnostics::*, registry::GenerationRecord};
impl<R: RunnerRegistrations> RunnerOperation<'_, R> {
    pub(super) fn capture(&self, generation: &GenerationRecord, now: i64) -> Capture {
        let Some(guard) = self.diagnostic_guard else {
            return Capture(None);
        };
        let mut guard = Guard::fleet(&generation.fleet_key, guard);
        guard.bind_generation(generation);
        Capture::start(
            self.store.diagnostic_sink(),
            guard,
            Lane::Cleanup,
            QuestionId::Cleanup,
            now,
        )
    }
}
