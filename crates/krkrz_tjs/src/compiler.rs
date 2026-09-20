use crate::{
    Instruction, Op, Program, Value,
    lexer::{Kind, Token, lex},
};
use anyhow::{Result, bail, ensure};
use krkrz_core::SourceLocation;
#[derive(Debug)]
enum Expr {
    Value(Value),
    Name(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Assign(String, Box<Expr>),
    Call(String, Vec<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
}
struct Compiler {
    tokens: Vec<Token>,
    pos: usize,
    program: Program,
    depth: usize,
    expr_nodes: usize,
}
impl Compiler {
    fn at(&self) -> SourceLocation {
        self.tokens[self.pos].location.clone()
    }
    fn is(&self, s: &str) -> bool {
        matches!(&self.tokens[self.pos].kind,Kind::Name(n)|Kind::Symbol(n) if n==s)
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<()> {
        ensure!(
            self.eat(s),
            "expected {s:?}, found {:?}",
            self.tokens[self.pos].kind
        );
        Ok(())
    }
    fn name(&mut self) -> Result<String> {
        match self.tokens[self.pos].kind.clone() {
            Kind::Name(n) => {
                self.pos += 1;
                Ok(n)
            }
            _ => bail!("expected TJS identifier"),
        }
    }
    fn eof(&self) -> bool {
        self.tokens[self.pos].kind == Kind::Eof
    }
    fn reg(&mut self) -> usize {
        let r = self.program.registers;
        self.program.registers += 1;
        r
    }
    fn emit(&mut self, op: Op, at: &SourceLocation) -> usize {
        let i = self.program.code.len();
        self.program.code.push(Instruction {
            location: at.clone(),
            op,
        });
        i
    }
    fn patch(&mut self, i: usize) {
        let target = self.program.code.len();
        match &mut self.program.code[i].op {
            Op::Jump { target: t } | Op::JumpUnless { target: t, .. } => *t = target,
            _ => unreachable!(),
        }
    }
    fn expression(&mut self, min: u8) -> Result<Expr> {
        self.depth += 1;
        self.expr_nodes += 1;
        ensure!(
            self.depth <= 128 && self.expr_nodes <= 512,
            "TJS expression complexity exceeds limit"
        );
        let mut left = if self.eat("(") {
            let e = self.expression(0)?;
            self.expect(")")?;
            e
        } else if ["!", "~", "+", "-", "typeof", "int", "real", "string"]
            .iter()
            .any(|op| self.is(op))
        {
            let op = match self.tokens[self.pos].kind.clone() {
                Kind::Name(s) | Kind::Symbol(s) => s,
                _ => unreachable!(),
            };
            self.pos += 1;
            Expr::Unary(op, Box::new(self.expression(14)?))
        } else {
            match self.tokens[self.pos].kind.clone() {
                Kind::Literal(v) => {
                    self.pos += 1;
                    Expr::Value(v)
                }
                Kind::Name(n) => {
                    ensure!(
                        ![
                            "class",
                            "function",
                            "property",
                            "new",
                            "null",
                            "try",
                            "throw",
                            "with",
                            "delete",
                            "invalidate",
                            "incontextof"
                        ]
                        .contains(&n.as_str()),
                        "unsupported TJS construct {n}"
                    );
                    self.pos += 1;
                    Expr::Name(n)
                }
                ref token => bail!("unsupported TJS expression token {token:?}"),
            }
        };
        loop {
            if self.eat(".") {
                let member = self.name()?;
                let Expr::Name(name) = left else {
                    bail!("object property dispatch is not implemented")
                };
                left = Expr::Name(format!("{name}.{member}"));
                continue;
            }
            if self.eat("(") {
                let Expr::Name(name) = left else {
                    bail!("closure invocation is not implemented")
                };
                let mut args = vec![];
                if !self.eat(")") {
                    loop {
                        args.push(self.expression(0)?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                left = Expr::Call(name, args);
                continue;
            }
            if min <= 2 && self.eat("?") {
                let yes = self.expression(0)?;
                self.expect(":")?;
                let no = self.expression(2)?;
                left = Expr::Conditional(Box::new(left), Box::new(yes), Box::new(no));
                continue;
            }
            let Kind::Symbol(op) = self.tokens[self.pos].kind.clone() else {
                break;
            };
            let precedence = match op.as_str() {
                "=" => 1,
                "||" => 3,
                "&&" => 4,
                "|" => 5,
                "^" => 6,
                "&" => 7,
                "==" | "!=" | "===" | "!==" => 8,
                "<" | ">" | "<=" | ">=" => 9,
                "<<" | ">>" | ">>>" => 10,
                "+" | "-" => 11,
                "*" | "/" | "%" | "\\" => 12,
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.pos += 1;
            let right = self.expression(if op == "=" {
                precedence
            } else {
                precedence + 1
            })?;
            left = if op == "=" {
                let Expr::Name(name) = left else {
                    bail!("invalid assignment target")
                };
                Expr::Assign(name, Box::new(right))
            } else {
                Expr::Binary(op, Box::new(left), Box::new(right))
            };
        }
        self.depth -= 1;
        Ok(left)
    }
    fn compile_expr(&mut self, e: Expr, at: &SourceLocation) -> usize {
        let out = self.reg();
        match e {
            Expr::Value(value) => {
                self.emit(Op::Constant { out, value }, at);
            }
            Expr::Name(name) => {
                self.emit(
                    Op::Load {
                        out,
                        name,
                        optional: false,
                    },
                    at,
                );
            }
            Expr::Assign(name, value) => {
                let input = self.compile_expr(*value, at);
                self.emit(Op::Store { name, input }, at);
                self.emit(Op::Move { out, input }, at);
            }
            Expr::Unary(op, e) => {
                let input = if op == "typeof" {
                    if let Expr::Name(name) = *e {
                        let r = self.reg();
                        self.emit(
                            Op::Load {
                                out: r,
                                name,
                                optional: true,
                            },
                            at,
                        );
                        r
                    } else {
                        self.compile_expr(*e, at)
                    }
                } else {
                    self.compile_expr(*e, at)
                };
                self.emit(Op::Unary { out, op, input }, at);
            }
            Expr::Binary(op, left, right) if op == "&&" || op == "||" => {
                let left = self.compile_expr(*left, at);
                self.emit(
                    Op::Unary {
                        out,
                        op: "!".into(),
                        input: left,
                    },
                    at,
                );
                self.emit(
                    Op::Unary {
                        out,
                        op: "!".into(),
                        input: out,
                    },
                    at,
                );
                let test = if op == "||" {
                    let r = self.reg();
                    self.emit(
                        Op::Unary {
                            out: r,
                            op: "!".into(),
                            input: out,
                        },
                        at,
                    );
                    r
                } else {
                    out
                };
                let jump = self.emit(
                    Op::JumpUnless {
                        input: test,
                        target: 0,
                    },
                    at,
                );
                let right = self.compile_expr(*right, at);
                self.emit(
                    Op::Unary {
                        out,
                        op: "!".into(),
                        input: right,
                    },
                    at,
                );
                self.emit(
                    Op::Unary {
                        out,
                        op: "!".into(),
                        input: out,
                    },
                    at,
                );
                self.patch(jump);
            }
            Expr::Binary(op, left, right) => {
                let left = self.compile_expr(*left, at);
                let right = self.compile_expr(*right, at);
                self.emit(
                    Op::Binary {
                        out,
                        op,
                        left,
                        right,
                    },
                    at,
                );
            }
            Expr::Call(name, args) => {
                let args = args.into_iter().map(|e| self.compile_expr(e, at)).collect();
                self.emit(Op::Call { out, name, args }, at);
            }
            Expr::Conditional(test, yes, no) => {
                let test = self.compile_expr(*test, at);
                let jump = self.emit(
                    Op::JumpUnless {
                        input: test,
                        target: 0,
                    },
                    at,
                );
                let input = self.compile_expr(*yes, at);
                self.emit(Op::Move { out, input }, at);
                let end = self.emit(Op::Jump { target: 0 }, at);
                self.patch(jump);
                let input = self.compile_expr(*no, at);
                self.emit(Op::Move { out, input }, at);
                self.patch(end);
            }
        }
        out
    }
    fn end(&mut self) -> Result<()> {
        ensure!(
            self.eat(";") || self.is("}") || self.eof(),
            "expected semicolon; unsupported TJS statement continuation"
        );
        Ok(())
    }
    fn statement(&mut self) -> Result<()> {
        self.expr_nodes = 0;
        self.depth += 1;
        ensure!(self.depth <= 128, "TJS statement nesting exceeds limit");
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }
    fn statement_inner(&mut self) -> Result<()> {
        let at = self.at();
        if self.eat(";") {
            return Ok(());
        }
        if self.eat("{") {
            while !self.eat("}") {
                ensure!(!self.eof(), "unterminated TJS block");
                self.statement()?;
            }
            return Ok(());
        }
        if self.eat("var") {
            loop {
                let name = self.name()?;
                let e = if self.eat("=") {
                    self.expression(0)?
                } else {
                    Expr::Value(Value::Void)
                };
                let input = self.compile_expr(e, &at);
                self.emit(Op::Store { name, input }, &at);
                if !self.eat(",") {
                    break;
                }
            }
            return self.end();
        }
        if self.eat("if") {
            self.expect("(")?;
            let expr = self.expression(0)?;
            self.expect(")")?;
            let input = self.compile_expr(expr, &at);
            let jump = self.emit(Op::JumpUnless { input, target: 0 }, &at);
            self.statement()?;
            if self.eat("else") {
                let end = self.emit(Op::Jump { target: 0 }, &at);
                self.patch(jump);
                self.statement()?;
                self.patch(end);
            } else {
                self.patch(jump);
            }
            return Ok(());
        }
        if self.eat("while") {
            let top = self.program.code.len();
            self.expect("(")?;
            let expr = self.expression(0)?;
            self.expect(")")?;
            let input = self.compile_expr(expr, &at);
            let end = self.emit(Op::JumpUnless { input, target: 0 }, &at);
            self.statement()?;
            self.emit(Op::Jump { target: top }, &at);
            self.patch(end);
            return Ok(());
        }
        if self.eat("return") {
            let expr = if self.is(";") || self.is("}") || self.eof() {
                Expr::Value(Value::Void)
            } else {
                self.expression(0)?
            };
            let input = self.compile_expr(expr, &at);
            self.emit(Op::Return { input }, &at);
            return self.end();
        }
        let expr = self.expression(0)?;
        self.compile_expr(expr, &at);
        self.end()
    }
}
fn run_compile(storage: &str, source: &str, expression: bool) -> Result<Program> {
    let mut c = Compiler {
        tokens: lex(storage, source)?,
        pos: 0,
        program: Program {
            storage: storage.into(),
            registers: 0,
            code: vec![],
        },
        depth: 0,
        expr_nodes: 0,
    };
    let result = (|| -> Result<()> {
        if expression {
            let at = c.at();
            let e = c.expression(0)?;
            ensure!(c.eof(), "unexpected tokens after TJS expression");
            let input = c.compile_expr(e, &at);
            c.emit(Op::Return { input }, &at);
        } else {
            while !c.eof() {
                c.statement()?;
            }
        }
        Ok(())
    })();
    result.map_err(|e| {
        let at = c.at();
        anyhow::anyhow!("{}:{}:{}: {e}", at.storage, at.line, at.column)
    })?;
    c.program.validate()?;
    Ok(c.program)
}
pub fn compile(storage: &str, source: &str) -> Result<Program> {
    run_compile(storage, source, false)
}
pub fn compile_expression(storage: &str, source: &str) -> Result<Program> {
    run_compile(storage, source, true)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vm;
    #[test]
    fn register_execution_and_short_circuit() {
        let p = compile(
            "synthetic.tjs",
            "var x=0; while(x<4){x=x+1;} if(0 && missing()) x=99; return x;",
        )
        .unwrap();
        let mut vm = Vm::default();
        assert_eq!(
            vm.execute(&p, &mut (), &mut 1000).unwrap(),
            Value::Integer(4)
        );
        assert_eq!(
            vm.execute(
                &compile_expression("expr", "1 || missing()").unwrap(),
                &mut (),
                &mut 1000
            )
            .unwrap(),
            Value::Integer(1)
        );
        assert_eq!(
            vm.execute(
                &compile_expression("expr", "0 ? missing() : 7").unwrap(),
                &mut (),
                &mut 1000
            )
            .unwrap(),
            Value::Integer(7)
        );
    }
    #[test]
    fn bounded_errors_and_locations() {
        let p = compile("loop.tjs", "while(1) {}").unwrap();
        let e = Vm::default().execute(&p, &mut (), &mut 10).unwrap_err();
        assert!(e.to_string().contains("loop.tjs:1:1"));
        assert!(
            compile("class.tjs", "class A {}")
                .unwrap_err()
                .to_string()
                .contains("unsupported TJS construct class")
        );
    }
}
