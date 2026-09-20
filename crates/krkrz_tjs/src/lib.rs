//! TJS values, objects, source compiler, and a bounded register interpreter.
//! Unsupported source constructs are errors; this is not a JavaScript adapter.
mod classes;
mod compiler;
mod dispatch;
pub mod lexer;
mod member_layout;
mod object;
mod preprocessor;
mod regexp;
mod scripts_ex;
pub use preprocessor::Preprocessor;
pub mod value;
mod vm;
pub use compiler::{compile, compile_expression, compile_with_preprocessor};
pub use object::{Class, Function, Property, VmAbort, unsupported};
pub use value::{ObjectRef, Value};
pub use vm::{Argument, Host, Instruction, Op, Program, Vm};
