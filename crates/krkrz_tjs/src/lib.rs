//! TJS values, objects, source compiler, and a bounded register interpreter.
//! Unsupported source constructs are errors; this is not a JavaScript adapter.
mod array_sort;
mod classes;
mod compiler;
mod date;
mod dispatch;
mod gc;
pub mod lexer;
mod lifetime;
mod literal;
mod math;
mod member_layout;
mod missing;
pub use literal::Literal;
mod object;
mod preprocessor;
mod readonly;
mod regexp;
pub use readonly::ReadOnlyData;
mod scripts_ex;
mod serialization;
mod string_format;
pub use preprocessor::Preprocessor;
pub mod value;
mod vm;
pub use compiler::{compile, compile_expression, compile_with_preprocessor};
pub use object::{Class, Function, Property, VmAbort, unsupported};
pub use value::{ObjectRef, Value};
pub use vm::{Argument, Host, Instruction, Op, Program, Vm};

mod random_generator;
