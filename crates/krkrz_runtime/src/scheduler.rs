use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventKind {
    Trigger,
    Timer,
}
impl EventKind {
    pub(crate) fn method(self) -> &'static str {
        match self {
            Self::Trigger => "onFire",
            Self::Timer => "onTimer",
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct PostedEvent {
    pub target: usize,
    pub kind: EventKind,
    sequence: u64,
    priority: i32,
}
#[derive(Default)]
pub(crate) struct EventQueue {
    pending: std::collections::VecDeque<PostedEvent>,
    sequence: u64,
    exclusive_posted: bool,
}
impl EventQueue {
    pub fn cancel(&mut self, target: usize, kind: EventKind) {
        self.pending
            .retain(|e| e.target != target || e.kind != kind);
    }
    pub fn count(&self, target: usize, kind: EventKind) -> usize {
        self.pending
            .iter()
            .filter(|e| e.target == target && e.kind == kind)
            .count()
    }
    pub fn begin_batch(&mut self) -> u64 {
        self.exclusive_posted = false;
        self.sequence
    }
    pub fn exclusive_posted(&self) -> bool {
        self.exclusive_posted
    }
    pub fn post(&mut self, target: usize, kind: EventKind, mode: i32) -> Result<()> {
        if self.pending.len() >= 100_000 {
            return Err(krkrz_tjs::unsupported("pending event limit exceeded"));
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| krkrz_tjs::unsupported("event sequence overflow"))?;
        let priority = match mode {
            1 => 1,
            2 => 2,
            _ => 0,
        };
        self.pending.push_back(PostedEvent {
            target,
            kind,
            priority,
            sequence: self.sequence,
        });
        self.exclusive_posted |= priority == 1;
        Ok(())
    }
    pub fn pop(&mut self, through: u64, priority: i32) -> Option<PostedEvent> {
        let index = self
            .pending
            .iter()
            .position(|e| e.sequence <= through && e.priority == priority)?;
        self.pending.remove(index)
    }
}
/// Stable order for simultaneous timer and transition completion callbacks.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Scheduler {
    now: u64,
    next_id: u64,
    pending: BTreeMap<(u64, u64), String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Callback {
    pub id: u64,
    pub time_ms: u64,
    pub name: String,
}
impl Scheduler {
    pub fn schedule(&mut self, delay_ms: u64, name: String) -> Result<u64> {
        let time = self
            .now
            .checked_add(delay_ms)
            .ok_or_else(|| anyhow::anyhow!("timer deadline overflow"))?;
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("timer ID overflow"))?;
        self.pending.insert((time, id), name);
        Ok(id)
    }
    pub fn cancel(&mut self, id: u64) {
        self.pending.retain(|(_, key), _| *key != id);
    }
    pub fn advance(&mut self, time_ms: u64) -> Result<Vec<Callback>> {
        ensure!(time_ms >= self.now, "clock cannot move backwards");
        let mut out = vec![];
        while self
            .pending
            .first_key_value()
            .is_some_and(|((time, _), _)| *time <= time_ms)
        {
            let ((time, id), name) = self.pending.pop_first().unwrap();
            out.push(Callback {
                id,
                time_ms: time,
                name,
            });
        }
        self.now = time_ms;
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_callbacks() {
        let mut s = Scheduler::default();
        s.schedule(10, "first".into()).unwrap();
        let cancel = s.schedule(5, "cancel".into()).unwrap();
        s.schedule(10, "second".into()).unwrap();
        s.cancel(cancel);
        assert!(s.advance(9).unwrap().is_empty());
        assert_eq!(
            s.advance(10)
                .unwrap()
                .into_iter()
                .map(|c| c.name)
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(s.advance(1).is_err());
    }
}
