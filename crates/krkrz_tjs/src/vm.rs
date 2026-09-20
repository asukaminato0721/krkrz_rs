use crate::Value;
use anyhow::{Context, Result, bail, ensure};
use krkrz_core::SourceLocation;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    Constant {
        out: usize,
        value: Value,
    },
    Load {
        out: usize,
        name: String,
        optional: bool,
    },
    Store {
        name: String,
        input: usize,
    },
    Move {
        out: usize,
        input: usize,
    },
    Unary {
        out: usize,
        op: String,
        input: usize,
    },
    Binary {
        out: usize,
        op: String,
        left: usize,
        right: usize,
    },
    Jump {
        target: usize,
    },
    JumpUnless {
        input: usize,
        target: usize,
    },
    Call {
        out: usize,
        name: String,
        args: Vec<usize>,
    },
    Return {
        input: usize,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instruction {
    pub location: SourceLocation,
    pub op: Op,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Program {
    pub storage: String,
    pub registers: usize,
    pub code: Vec<Instruction>,
}
impl Program {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.registers <= 1_000_000 && self.code.len() <= 1_000_000,
            "TJS program exceeds limit"
        );
        for i in &self.code {
            let regs = match &i.op {
                Op::Constant { out, .. } | Op::Load { out, .. } => vec![*out],
                Op::Store { input, .. } | Op::Return { input } | Op::JumpUnless { input, .. } => {
                    vec![*input]
                }
                Op::Move { out, input } | Op::Unary { out, input, .. } => vec![*out, *input],
                Op::Binary {
                    out, left, right, ..
                } => vec![*out, *left, *right],
                Op::Call { out, args, .. } => {
                    let mut r = args.clone();
                    r.push(*out);
                    r
                }
                Op::Jump { .. } => vec![],
            };
            ensure!(
                regs.into_iter().all(|r| r < self.registers),
                "invalid TJS register operand"
            );
            if let Op::Jump { target } | Op::JumpUnless { target, .. } = i.op {
                ensure!(target <= self.code.len(), "invalid TJS branch target");
            }
        }
        Ok(())
    }
}
pub trait Host {
    fn call(&mut self, vm: &mut Vm, name: &str, args: &[Value], budget: &mut u64) -> Result<Value>;
    fn trace(&mut self, _instruction: &Instruction) -> Result<()> {
        Ok(())
    }
}
impl Host for () {
    fn call(
        &mut self,
        _vm: &mut Vm,
        name: &str,
        _args: &[Value],
        _budget: &mut u64,
    ) -> Result<Value> {
        bail!("unsupported native call: {name}")
    }
}
#[derive(Default)]
pub struct Vm {
    pub globals: BTreeMap<String, Value>,
    pub executed: u64,
}
impl Vm {
    /// The budget is shared by repeated executions through the caller's mutable counter.
    pub fn execute(
        &mut self,
        program: &Program,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        program.validate()?;
        let mut registers = vec![Value::Void; program.registers];
        let mut ip = 0;
        while let Some(instruction) = program.code.get(ip) {
            let at = &instruction.location;
            ensure!(
                *budget > 0,
                "{}:{}:{}: execution budget exhausted after {} instructions",
                at.storage,
                at.line,
                at.column,
                self.executed
            );
            *budget -= 1;
            self.executed += 1;
            host.trace(instruction)?;
            ip += 1;
            let result = (|| -> Result<Option<Value>> {
                match &instruction.op {
                    Op::Constant { out, value } => registers[*out] = value.clone(),
                    Op::Load {
                        out,
                        name,
                        optional,
                    } => {
                        registers[*out] = if *optional {
                            self.globals.get(name).cloned().unwrap_or(Value::Void)
                        } else {
                            self.globals
                                .get(name)
                                .with_context(|| format!("member not found: {name}"))?
                                .clone()
                        }
                    }
                    Op::Store { name, input } => {
                        self.globals.insert(name.clone(), registers[*input].clone());
                    }
                    Op::Move { out, input } => registers[*out] = registers[*input].clone(),
                    Op::Unary { out, op, input } => {
                        registers[*out] = registers[*input].unary(op)?
                    }
                    Op::Binary {
                        out,
                        op,
                        left,
                        right,
                    } => registers[*out] = registers[*left].binary(op, &registers[*right])?,
                    Op::Jump { target } => ip = *target,
                    Op::JumpUnless { input, target } => {
                        if !registers[*input].truth()? {
                            ip = *target;
                        }
                    }
                    Op::Call { out, name, args } => {
                        registers[*out] = host.call(
                            self,
                            name,
                            &args
                                .iter()
                                .map(|i| registers[*i].clone())
                                .collect::<Vec<_>>(),
                            budget,
                        )?
                    }
                    Op::Return { input } => return Ok(Some(registers[*input].clone())),
                }
                Ok(None)
            })();
            if let Some(value) = result.with_context(|| {
                format!(
                    "{}:{}:{} at VM instruction {}",
                    at.storage,
                    at.line,
                    at.column,
                    ip.saturating_sub(1)
                )
            })? {
                return Ok(value);
            }
        }
        Ok(Value::Void)
    }
}
