//! Math.RandomGenerator's MT19937 stream and reconstructible state.
//! Port of tjsRandomGenerator.cpp and tjsMT19937ar-cok.cpp; see THIRD_PARTY_NOTICES.
use crate::{Host, Value, Vm};
use anyhow::{Context, Result, ensure};
const N: usize = 624;
#[derive(Clone, Debug)]
pub(crate) struct Generator {
    state: [u32; N],
    left: usize,
    next: usize,
}
impl Generator {
    fn seed(seed: u64) -> Self {
        let keys = [seed as u32, (seed >> 32) as u32];
        let mut result = Self {
            state: [0; N],
            left: 1,
            next: 0,
        };
        result.state[0] = 19650218;
        for i in 1..N {
            result.state[i] = 1812433253u32
                .wrapping_mul(result.state[i - 1] ^ (result.state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        let mut i = 1;
        let mut j = 0;
        for _ in 0..N {
            result.state[i] = (result.state[i]
                ^ (result.state[i - 1] ^ (result.state[i - 1] >> 30)).wrapping_mul(1664525))
            .wrapping_add(keys[j])
            .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i == N {
                result.state[0] = result.state[N - 1];
                i = 1;
            }
            if j == keys.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            result.state[i] = (result.state[i]
                ^ (result.state[i - 1] ^ (result.state[i - 1] >> 30)).wrapping_mul(1566083941))
            .wrapping_sub(i as u32);
            i += 1;
            if i == N {
                result.state[0] = result.state[N - 1];
                i = 1;
            }
        }
        result.state[0] = 0x80000000;
        result
    }
    fn word(&mut self) -> u32 {
        self.left -= 1;
        if self.left == 0 {
            for i in 0..N {
                let mixed = (self.state[i] & 0x80000000) | (self.state[(i + 1) % N] & 0x7fffffff);
                self.state[i] = self.state[(i + 397) % N]
                    ^ (mixed >> 1)
                    ^ if mixed & 1 != 0 { 0x9908b0df } else { 0 };
            }
            self.left = N;
            self.next = 0;
        }
        let mut y = self.state[self.next];
        self.next += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;
        y
    }
    fn wide(&mut self) -> u64 {
        u64::from(self.word()) | (u64::from(self.word()) << 32)
    }
}
impl Vm {
    pub(crate) fn register_random_generator(&mut self, math: &Value) -> Result<()> {
        let class = self.new_native_class("RandomGenerator", "RandomGenerator.@initialize")?;
        for name in [
            "RandomGenerator",
            "randomize",
            "random",
            "random32",
            "random63",
            "random64",
            "serialize",
            "finalize",
        ] {
            self.register_native_method(&class, name, &format!("RandomGenerator.{name}"))?;
        }
        self.set_member(math, &Value::string("RandomGenerator"), class)
    }
    pub(crate) fn random_generator_call(
        &mut self,
        operation: &str,
        context: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let id = self.object_id(context)?;
        ensure!(
            self.objects[id]
                .classes
                .iter()
                .any(|name| name == "RandomGenerator"),
            "context has no RandomGenerator instance"
        );
        match operation {
            "RandomGenerator" | "randomize" => {
                let generator = if let Some(value) = args.first() {
                    if matches!(value, Value::Object(_)) {
                        let text = self
                            .get_property(
                                value,
                                &Value::string("state"),
                                false,
                                false,
                                host,
                                budget,
                            )?
                            .text();
                        ensure!(
                            text.len() == N * 8 && text.is_ascii(),
                            "invalid RandomGenerator state"
                        );
                        let mut state = [0; N];
                        for (word, hex) in state
                            .iter_mut()
                            .zip(text.as_bytes().as_chunks::<8>().0.iter())
                        {
                            *word = u32::from_str_radix(std::str::from_utf8(hex)?, 16)
                                .context("invalid RandomGenerator state")?;
                        }
                        let left = self
                            .get_property(
                                value,
                                &Value::string("left"),
                                false,
                                false,
                                host,
                                budget,
                            )?
                            .integer()?;
                        let next = self
                            .get_property(
                                value,
                                &Value::string("next"),
                                false,
                                false,
                                host,
                                budget,
                            )?
                            .integer()?;
                        ensure!(
                            (1..=N as i64).contains(&left)
                                && (0..=N as i64).contains(&next)
                                && (next == 0 && left == 1 || next + left == N as i64 + 1),
                            "invalid RandomGenerator cursor"
                        );
                        Generator {
                            state,
                            left: left as usize,
                            next: next as usize,
                        }
                    } else {
                        Generator::seed(value.integer()? as u64)
                    }
                } else {
                    // Entropy is supplied by the session's replayable random stream.
                    let low = (self.math_call("random", &[], true)?.real()? * 4294967296.0) as u64;
                    let high = (self.math_call("random", &[], true)?.real()? * 4294967296.0) as u64;
                    Generator::seed(low | (high << 32))
                };
                self.objects[id].random_generator = Some(Box::new(generator));
                Ok(Value::Void)
            }
            "serialize" => {
                let generator = self.objects[id]
                    .random_generator
                    .as_ref()
                    .context("RandomGenerator is not initialized")?;
                let state = generator
                    .state
                    .iter()
                    .map(|n| format!("{n:08x}"))
                    .collect::<String>();
                let left = generator.left;
                let next = generator.next;
                let value = self.new_dictionary()?;
                self.set_member(&value, &Value::string("state"), Value::string(&state))?;
                self.set_member(&value, &Value::string("left"), Value::Integer(left as i64))?;
                self.set_member(&value, &Value::string("next"), Value::Integer(next as i64))?;
                Ok(value)
            }
            "finalize" => Ok(Value::Void),
            _ => {
                let generator = self.objects[id]
                    .random_generator
                    .as_mut()
                    .context("RandomGenerator is not initialized")?;
                Ok(match operation {
                    "random32" => Value::Integer(generator.word().into()),
                    "random63" => Value::Integer((generator.wide() & 0x7fffffffffffffff) as i64),
                    "random64" => Value::Integer(generator.wide() as i64),
                    "random" => Value::Real(
                        f64::from_bits((generator.wide() & 0xfffffffffffff) | 0x3ff0000000000000)
                            - 1.0,
                    ),
                    _ => anyhow::bail!("unknown RandomGenerator operation: {operation}"),
                })
            }
        }
    }
}
