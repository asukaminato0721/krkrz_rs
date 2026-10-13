//! Timer bindings and 16.16 millisecond deadlines, following Kirikiri TimerIntf/Impl.
use crate::coroutine::TaskId;
use crate::{Services, scheduler::EventKind};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};

const SCALE: u64 = 1 << 16;
pub(crate) struct Timer {
    constructed: bool,
    owner: Value,
    action: Value,
    interval: u64,
    next: u64,
    enabled: bool,
    capacity: i32,
    mode: i32,
    task: Option<TaskId>,
}
impl Default for Timer {
    fn default() -> Self {
        Self {
            constructed: false,
            owner: Value::NULL,
            action: Value::string("action"),
            interval: 1000,
            next: 0,
            enabled: false,
            capacity: 6,
            mode: 0,
            task: None,
        }
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("Timer")?;
    for method in ["Timer", "finalize", "onTimer"] {
        vm.register_native_method(&class, method, &format!("Timer.{method}"))?;
    }
    for property in ["interval", "enabled", "capacity", "mode"] {
        vm.register_native_property(
            &class,
            property,
            Some(&format!("Timer.get:{property}")),
            Some(&format!("Timer.set:{property}")),
        )?;
    }
    Ok(())
}
fn ticks(time_ms: u64) -> Result<u64> {
    time_ms
        .checked_mul(SCALE)
        .ok_or_else(|| unsupported("Timer clock range exceeded"))
}
impl Services {
    pub(crate) fn timer_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            anyhow::bail!("Timer requires an Object context");
        };
        let id = reference
            .object
            .context("Timer requires a non-null context")?;
        if operation == "@initialize" {
            self.timers.entry(id).or_default();
            return Ok(Value::Void);
        }
        if operation == "@invalidate" {
            self.events.cancel(id, EventKind::Timer);
            if let Some(timer) = self.timers.remove(&id)
                && let Some(task) = timer.task
            {
                self.timer_tasks.cancel(task);
            }
            return Ok(Value::Void);
        }
        let timer = self
            .timers
            .get_mut(&id)
            .context("context has no Timer native instance")?;
        let arg = |index: usize| {
            args.get(index)
                .with_context(|| format!("Timer.{operation}: missing argument {index}"))
        };
        let mut rearm = false;
        if operation == "finalize" {
            return Ok(Value::Void);
        }
        if operation == "Timer" {
            // TimerImpl ignores BaseTimer's missing-argument return code.
            if let Some(owner) = args.first() {
                if let Some(name) = args.get(1).filter(|v| !matches!(v, Value::Void)) {
                    timer.action = name.unary("string")?;
                }
                ensure!(
                    matches!(owner, Value::Object(_)),
                    "Timer action owner must be an Object"
                );
                timer.owner = owner.clone();
            }
            timer.constructed = true;
            return Ok(Value::Void);
        }
        ensure!(timer.constructed, "Timer constructor has not run");
        match operation {
            "get:interval" => return Ok(Value::Real(timer.interval as f64 / SCALE as f64)),
            "get:enabled" => return Ok(Value::Integer(timer.enabled.into())),
            "get:capacity" => return Ok(Value::Integer(timer.capacity.into())),
            "get:mode" => return Ok(Value::Integer(timer.mode.into())),
            "set:interval" => {
                let value = arg(0)?.real()? * SCALE as f64 + 0.5;
                // The reference conversion returns INT64_MIN for an out-of-range double.
                let value = if value.is_finite()
                    && (-9223372036854775808.0..9223372036854775808.0).contains(&value)
                {
                    value as i64
                } else {
                    i64::MIN
                };
                timer.interval = value as u64;
                if timer.enabled {
                    self.events.cancel(id, EventKind::Timer);
                    timer.next = ticks(self.time_ms)?.wrapping_add(timer.interval);
                    rearm = true;
                }
            }
            "set:enabled" => {
                timer.enabled = arg(0)?.truth()?;
                if timer.enabled {
                    timer.next = ticks(self.time_ms)?.wrapping_add(timer.interval);
                } else {
                    self.events.cancel(id, EventKind::Timer);
                }
                rearm = true;
            }
            "set:capacity" => timer.capacity = arg(0)?.integer()? as i32,
            "set:mode" => timer.mode = arg(0)?.integer()? as i32,
            "onTimer" => {
                let owner = timer.owner.clone();
                let action = timer.action.clone();
                if matches!(owner, Value::Object(ObjectRef { object: None, .. })) {
                    return Ok(Value::Void);
                }
                let event = vm.new_dictionary()?;
                vm.set_member(&event, &Value::string("type"), Value::string("onTimer"))?;
                vm.set_member(
                    &event,
                    &Value::string("target"),
                    Value::Object(ObjectRef {
                        object: Some(id),
                        context: Some(id),
                    }),
                )?;
                let callback = if action == Value::string("") {
                    owner.clone()
                } else {
                    vm.get_property(&owner, &action, true, false, self, budget)?
                };
                if callback == Value::Void {
                    return Ok(Value::Void);
                }
                return vm.call_function(&callback, &owner, &[event], self, budget);
            }
            _ => return Err(unsupported(format!("Timer.{operation}"))),
        }
        if rearm {
            self.arm_timer(id)?;
        }
        Ok(Value::Void)
    }

    fn arm_timer(&mut self, id: usize) -> Result<()> {
        let timer = self.timers.get_mut(&id).context("timer no longer exists")?;
        if let Some(task) = timer.task.take() {
            self.timer_tasks.cancel(task);
        }
        if timer.constructed && timer.enabled && timer.interval != 0 {
            // Kirikiri fires strictly after the 16.16 deadline, not at equality.
            let deadline = timer.next.saturating_add(1);
            timer.task = Some(self.timer_tasks.spawn(move |cx| async move {
                cx.wait_until(deadline).await;
                id
            })?);
        }
        Ok(())
    }

    /// Sample the native timer thread at the host's deterministic clock value.
    pub(crate) fn advance_timers(&mut self, time_ms: u64) -> Result<()> {
        let now = ticks(time_ms)?;
        let lax = self.arguments.get("-laxtimer").is_some_and(|v| v == "yes");
        // Preserve object order for simultaneous timers, regardless of task order.
        let mut due = self.timer_tasks.advance(now)?;
        due.sort_unstable_by_key(|(_, id)| *id);
        let mut due = due.into_iter();
        while let Some((_, id)) = due.next() {
            if let Err(error) = self.post_due_timer(id, now, lax) {
                // An event queue error must not discard other completed waits.
                // Recreate those waits with their unchanged timer deadlines.
                for (_, id) in due {
                    self.arm_timer(id)?;
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn post_due_timer(&mut self, id: usize, now: u64, lax: bool) -> Result<()> {
        let timer = self
            .timers
            .get_mut(&id)
            .context("timer task has no owner")?;
        timer.task = None;
        let missed = (now - timer.next) as u128 / timer.interval as u128 + 1;
        let count = if missed > 40 {
            timer.next = now.wrapping_add(timer.interval);
            1
        } else {
            timer.next = timer
                .next
                .wrapping_add((missed as u64).wrapping_mul(timer.interval));
            missed as usize
        };
        let capacity = if lax {
            1
        } else if timer.capacity == 0 {
            65535
        } else {
            timer.capacity.max(0) as usize
        };
        let available = capacity.saturating_sub(self.events.count(id, EventKind::Timer));
        let mode = timer.mode;
        self.arm_timer(id)?;
        for _ in 0..count.min(available) {
            self.events.post(id, EventKind::Timer, mode)?;
        }
        Ok(())
    }
}

impl Timer {
    pub(crate) fn gc_trace(&self, out: &mut Vec<Value>) {
        out.extend([self.owner.clone(), self.action.clone()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    #[test]
    fn event_queue_failure_keeps_unprocessed_timer_waits() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        session.evaluate(
            "(global.a=new Timer(null),global.b=new Timer(null),a.interval=1,b.interval=1,a.enabled=true,b.enabled=true)",
        ).unwrap();
        let ids: Vec<_> = session.services.timers.keys().copied().collect();
        for _ in 0..100_000 {
            session
                .services
                .events
                .post(usize::MAX, EventKind::Trigger, 0)
                .unwrap();
        }
        assert!(
            session
                .services
                .advance_timers(2)
                .unwrap_err()
                .to_string()
                .contains("pending event limit")
        );
        session
            .services
            .events
            .cancel(usize::MAX, EventKind::Trigger);
        session.services.advance_timers(2).unwrap();
        assert_eq!(session.services.events.count(ids[0], EventKind::Timer), 0);
        assert_eq!(session.services.events.count(ids[1], EventKind::Timer), 2);
    }
}
