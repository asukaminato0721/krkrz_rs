//! Continuous callbacks follow base/EventIntf.cpp and run before window paint.
use crate::Session;
use anyhow::Result;
use krkrz_tjs::{Value, unsupported};

impl Session {
    fn dispatch_continuous_inner(&mut self) -> Result<()> {
        let mut index = 0;
        // Use live slots: removals suppress pending handlers, and new handlers
        // appended during a callback run in this same tick, as upstream does.
        while index < self.services.continuous_handlers.len() {
            if self.services.events.exclusive_posted() {
                break;
            }
            self.budget = self
                .budget
                .checked_sub(1)
                .ok_or_else(|| unsupported("continuous callback execution budget exceeded"))?;
            if let Some(handler) = self.services.continuous_handlers[index].clone() {
                let result = self.vm.call_event_handler(
                    &handler,
                    &Value::object(0),
                    &[Value::Integer(self.services.time_ms as i64)],
                    &mut self.services,
                    &mut self.budget,
                );
                if !matches!(result, Ok(true)) {
                    self.services.continuous_handlers[index] = None;
                }
                result?;
            }
            index += 1;
        }
        Ok(())
    }
    pub(crate) fn dispatch_continuous(&mut self) -> Result<()> {
        let result = self.dispatch_continuous_inner();
        self.services.continuous_handlers.retain(Option::is_some);
        result
    }
}
