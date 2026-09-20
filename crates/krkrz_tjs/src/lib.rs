//! TJS values, objects, source compiler, and a bounded register interpreter.
//! Unsupported source constructs are errors; this is not a JavaScript adapter.
mod classes;
mod compiler;
mod dispatch;
pub mod lexer;
mod object;
pub mod value;
mod vm;
pub use compiler::{compile, compile_expression};
pub use object::{Class, Function, Property, VmAbort, unsupported};
pub use value::{ObjectRef, Value};
pub use vm::{Argument, Host, Instruction, Op, Program, Vm};
