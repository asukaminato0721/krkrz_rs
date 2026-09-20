use crate::{Host, Value, Vm, object::ObjectKind, unsupported};
use anyhow::{Result, ensure};

impl Vm {
    pub(crate) fn array_sort(
        &mut self,
        receiver: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let id = self.object_id(receiver)?;
        let ObjectKind::Array(original) = &self.objects[id].kind else {
            anyhow::bail!("Array.sort requires an Array receiver");
        };
        *budget = budget
            .checked_sub(original.len() as u64)
            .ok_or_else(|| unsupported("Array.sort execution budget exceeded"))?;
        let mut sorted = original.clone();
        let callback = args
            .first()
            .filter(|v| matches!(v, Value::Object(_)))
            .cloned();
        let order = args
            .first()
            .filter(|v| !matches!(v, Value::Void | Value::Object(_)))
            .map(|v| v.unary("string"))
            .transpose()?;
        let order = match order {
            Some(Value::String(s)) => s.first().copied().unwrap_or(0) as u8,
            _ => b'+',
        };
        // A stable result also satisfies a request without the stability guarantee.
        if let Some(stable) = args.get(1).filter(|v| !matches!(v, Value::Void)) {
            stable.truth()?;
        }
        let mut scratch = sorted.clone();
        let n = sorted.len();
        // Fallible, bounded merge sort. Inconsistent script comparators cannot
        // panic the Rust sorter or cause an unbounded partition loop.
        let mut width = 1;
        while width < n {
            for begin in (0..n).step_by(width * 2) {
                let middle = (begin + width).min(n);
                let end = (middle + width).min(n);
                let (mut left, mut right) = (begin, middle);
                for output in &mut scratch[begin..end] {
                    *budget = budget
                        .checked_sub(1)
                        .ok_or_else(|| unsupported("Array.sort execution budget exceeded"))?;
                    let take_right = if left == middle {
                        true
                    } else if right == end {
                        false
                    } else {
                        let (a, b) = (&sorted[right], &sorted[left]);
                        if let Some(callback) = &callback {
                            self.invoke(
                                callback,
                                &Value::object(0),
                                &[a.clone(), b.clone()],
                                host,
                                budget,
                            )?
                            .truth()?
                        } else {
                            let (a, b) = match order {
                                b'0' | b'9'
                                    if matches!((a, b), (Value::String(_), Value::String(_))) =>
                                {
                                    (a.number()?, b.number()?)
                                }
                                b'a' | b'z' => (a.unary("string")?, b.unary("string")?),
                                _ => (a.clone(), b.clone()),
                            };
                            a.binary(
                                if matches!(order, b'-' | b'9' | b'z') {
                                    ">"
                                } else {
                                    "<"
                                },
                                &b,
                            )?
                            .truth()?
                        }
                    };
                    let index = if take_right {
                        let i = right;
                        right += 1;
                        i
                    } else {
                        let i = left;
                        left += 1;
                        i
                    };
                    *output = sorted[index].clone();
                }
            }
            std::mem::swap(&mut sorted, &mut scratch);
            width *= 2;
        }
        self.object_id(receiver)?; // A comparison callback may invalidate it.
        let ObjectKind::Array(current) = &mut self.objects[id].kind else {
            unreachable!()
        };
        ensure!(
            current.len() == n,
            "Array.sort comparison changed the array length"
        );
        *current = sorted;
        Ok(Value::Void)
    }
}
