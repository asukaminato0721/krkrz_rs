//! TJS primitive values, source lexer, and a bounded register interpreter.
//! Unsupported source constructs are errors; this is not a JavaScript adapter.
mod compiler;
pub mod lexer;
pub mod value;
mod vm;
pub use compiler::{compile, compile_expression};
pub use value::Value;
pub use vm::{Host, Instruction, Op, Program, Vm};
