use crate::{Class, Value, Vm};
use anyhow::{Result, ensure};
use std::f64::consts;

impl Vm {
    pub(crate) fn register_math(&mut self) -> Result<()> {
        let math = self.define_class(&Class {
            name: "Math".into(),
            bases: vec![],
            methods: vec![],
            properties: vec![],
            initializer: None,
        })?;
        self.globals.insert("Math".into(), math.clone());
        for method in [
            "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "exp", "floor", "log", "max",
            "min", "pow", "random", "round", "sin", "sqrt", "tan",
        ] {
            self.register_native_static_method(&math, method, &format!("Math.{method}"))?;
        }
        for name in [
            "E", "LOG2E", "LOG10E", "LN10", "LN2", "PI", "SQRT1_2", "SQRT2",
        ] {
            self.register_native_static_property(&math, name, Some(&format!("Math.{name}")), None)?;
        }
        Ok(())
    }

    /// Set the session's deterministic Math.random stream. Replay hosts can record
    /// and restore this state separately from script objects.
    pub fn set_random_state(&mut self, state: [u32; 4]) {
        self.random_state = state;
    }
    pub fn random_state(&self) -> [u32; 4] {
        self.random_state
    }

    pub(crate) fn math_call(
        &mut self,
        name: &str,
        args: &[Value],
        result_needed: bool,
    ) -> Result<Value> {
        let count = match name {
            "atan2" | "pow" => 2,
            "abs" | "acos" | "asin" | "atan" | "ceil" | "cos" | "exp" | "floor" | "log"
            | "round" | "sin" | "sqrt" | "tan" => 1,
            _ => 0,
        };
        ensure!(args.len() >= count, "Math.{name}: not enough arguments");
        if !result_needed {
            return Ok(Value::Void);
        }
        let value = match name {
            "E" => consts::E,
            "LOG2E" => consts::LOG2_E,
            "LOG10E" => consts::LOG10_E,
            "LN10" => consts::LN_10,
            "LN2" => consts::LN_2,
            "PI" => consts::PI,
            "SQRT1_2" => consts::SQRT_2 / 2.0,
            "SQRT2" => consts::SQRT_2,
            "random" => {
                // tTJSXorshift, used by Kirikiri Z since February 2015.
                let [x, y, z, w] = self.random_state;
                let t = x ^ (x << 11);
                let next = (w ^ (w >> 19)) ^ (t ^ (t >> 8));
                self.random_state = [y, z, w, next];
                next as f64 / 4294967296.0
            }
            "max" | "min" => {
                let maximum = name == "max";
                let mut result = if maximum {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                };
                for arg in args {
                    let n = arg.real()?;
                    if n.is_nan() {
                        return Ok(Value::Real(f64::NAN));
                    }
                    let preferred_zero = n == 0.0
                        && n.is_sign_negative() != maximum
                        && result.is_sign_negative() == maximum;
                    if preferred_zero || (maximum && n > result) || (!maximum && n < result) {
                        result = n;
                    }
                }
                result
            }
            _ => {
                let n = args[0].real()?;
                match name {
                    "abs" => n.abs(),
                    "acos" => n.acos(),
                    "asin" => n.asin(),
                    "atan" => n.atan(),
                    "atan2" => n.atan2(args[1].real()?),
                    "ceil" => n.ceil(),
                    "cos" => n.cos(),
                    "exp" => n.exp(),
                    "floor" => n.floor(),
                    "log" => n.ln(),
                    "pow" => n.powf(args[1].real()?),
                    "round" => (n + 0.5).floor(),
                    "sin" => n.sin(),
                    "sqrt" => n.sqrt(),
                    "tan" => n.tan(),
                    _ => unreachable!("registered Math operation"),
                }
            }
        };
        Ok(Value::Real(value))
    }
}
