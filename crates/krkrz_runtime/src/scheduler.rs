use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
