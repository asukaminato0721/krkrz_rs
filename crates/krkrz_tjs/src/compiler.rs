use crate::{
    Argument, Class, Function, Instruction, Op, Program, Property, Value,
    lexer::{Kind, Token, lex},
    unsupported,
};
use anyhow::{Result, bail, ensure};
use krkrz_core::SourceLocation;

#[derive(Debug)]
enum Expr {
    Value(Value),
    Name(String),
    With,
    ForwardArguments,
    Spread(Box<Expr>),
    Member(Box<Expr>, Box<Expr>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Assign(Box<Expr>, String, Box<Expr>),
    Update(Box<Expr>, bool, bool),
    Call(Box<Expr>, Vec<Expr>),
    Construct(Box<Expr>, Vec<Expr>),
    Eval(Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Array(Vec<Expr>),
    Dictionary(Vec<(Expr, Expr)>),
    Function(Box<Function>),
}
struct Loop {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    scopes: usize,
    handlers: usize,
    withs: usize,
}
struct Compiler {
    tokens: Vec<Token>,
    pos: usize,
    program: Program,
    depth: usize,
    expr_nodes: usize,
    scopes: usize,
    handlers: usize,
    withs: usize,
    loops: Vec<Loop>,
}
impl Compiler {
    fn at(&self) -> SourceLocation {
        self.tokens[self.pos].location.clone()
    }
    fn is(&self, s: &str) -> bool {
        matches!(&self.tokens[self.pos].kind, Kind::Name(n)|Kind::Symbol(n) if n==s)
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
    fn patch_to(&mut self, i: usize, target: usize) {
        match &mut self.program.code[i].op {
            Op::Jump { target: t }
            | Op::JumpUnless { target: t, .. }
            | Op::Try { target: t, .. } => *t = target,
            _ => unreachable!(),
        }
    }
    fn patch(&mut self, i: usize) {
        self.patch_to(i, self.program.code.len());
    }
    fn function(&mut self, name: String) -> Result<Function> {
        let mut parameters = vec![];
        let mut rest = None;
        if self.eat("(") && !self.eat(")") {
            loop {
                ensure!(parameters.len() < 1024, "too many TJS parameters");
                let parameter = self.name()?;
                if self.eat("*") {
                    rest = Some(parameter);
                    self.expect(")")?;
                    break;
                }
                parameters.push(parameter);
                if self.eat(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        ensure!(self.is("{"), "expected function body");
        let program = Program {
            storage: self.program.storage.clone(),
            registers: 0,
            code: vec![],
        };
        let parent = std::mem::replace(&mut self.program, program);
        let loops = std::mem::take(&mut self.loops);
        let (scopes, handlers, withs) = (self.scopes, self.handlers, self.withs);
        self.scopes = 1;
        self.handlers = 0;
        self.withs = 0;
        let result = self.statement();
        let program = std::mem::replace(&mut self.program, parent);
        self.loops = loops;
        self.scopes = scopes;
        self.handlers = handlers;
        self.withs = withs;
        result?;
        Ok(Function {
            name,
            parameters,
            rest,
            program,
        })
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
        } else if self.eat("function") {
            Expr::Function(Box::new(self.function("(anonymous)".into())?))
        } else if self.eat("[") {
            let mut items = vec![];
            while !self.eat("]") {
                ensure!(!self.eof(), "unterminated array");
                if self.eat(",") {
                    items.push(Expr::Value(Value::Void));
                    continue;
                }
                items.push(self.expression(0)?);
                if self.eat("]") {
                    break;
                }
                self.expect(",")?;
            }
            Expr::Array(items)
        } else if self.eat("%[") {
            let mut items = vec![];
            while !self.eat("]") {
                let key = self.expression(0)?;
                ensure!(
                    self.eat("=>") || self.eat(":"),
                    "expected dictionary key separator"
                );
                let value = self.expression(0)?;
                items.push((key, value));
                if self.eat("]") {
                    break;
                }
                self.expect(",")?;
            }
            Expr::Dictionary(items)
        } else if self.eat(".") {
            ensure!(self.withs > 0, "dot member outside with");
            Expr::Member(
                Box::new(Expr::With),
                Box::new(Expr::Value(Value::string(&self.name()?))),
            )
        } else if self.eat("++") {
            Expr::Update(Box::new(self.expression(14)?), true, true)
        } else if self.eat("--") {
            Expr::Update(Box::new(self.expression(14)?), false, true)
        } else if self.eat("new") {
            // Array/Dictionary/Exception constructors currently share call dispatch.
            let call = self.expression(14)?;
            let Expr::Call(callee, args) = call else {
                bail!("new requires a constructor call")
            };
            Expr::Construct(callee, args)
        } else if [
            "!", "~", "+", "-", "typeof", "int", "real", "string", "#", "$", "&", "*",
        ]
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
                    if [
                        "delete",
                        "invalidate",
                        "isvalid",
                        "instanceof",
                        "switch",
                        "do",
                    ]
                    .contains(&n.as_str())
                    {
                        return Err(unsupported(format!("unsupported TJS construct {n}")));
                    }
                    self.pos += 1;
                    Expr::Name(n)
                }
                ref token => bail!("unsupported TJS expression token {token:?}"),
            }
        };
        loop {
            if self.eat(".") {
                let key = Expr::Value(Value::string(&self.name()?));
                left = Expr::Member(Box::new(left), Box::new(key));
                continue;
            }
            if self.eat("[") {
                let key = self.expression(0)?;
                self.expect("]")?;
                left = Expr::Member(Box::new(left), Box::new(key));
                continue;
            }
            if self.eat("(") {
                let mut args = vec![];
                if !self.eat(")") {
                    loop {
                        ensure!(args.len() < 1024, "too many TJS arguments");
                        args.push(if self.eat("...") {
                            Expr::ForwardArguments
                        } else if self.is(",") {
                            Expr::Value(Value::Void)
                        } else {
                            self.expression(0)?
                        });
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                left = Expr::Call(Box::new(left), args);
                continue;
            }
            if self.eat("++") {
                left = Expr::Update(Box::new(left), true, false);
                continue;
            }
            if self.eat("--") {
                left = Expr::Update(Box::new(left), false, false);
                continue;
            }
            if self.eat("!") {
                left = Expr::Eval(Box::new(left));
                continue;
            }
            if min <= 2 && self.eat("?") {
                let yes = self.expression(0)?;
                self.expect(":")?;
                let no = self.expression(2)?;
                left = Expr::Conditional(Box::new(left), Box::new(yes), Box::new(no));
                continue;
            }
            let op = match self.tokens[self.pos].kind.clone() {
                Kind::Name(s) if s == "incontextof" || s == "instanceof" => s,
                Kind::Symbol(s) => s,
                _ => break,
            };
            let precedence = match op.as_str() {
                "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "\\=" | "&=" | "|=" | "^=" | "<<="
                | ">>=" | ">>>=" => 1,
                "||" => 3,
                "&&" => 4,
                "|" => 5,
                "^" => 6,
                "&" => 7,
                "==" | "!=" | "===" | "!==" => 8,
                "<" | ">" | "<=" | ">=" | "instanceof" => 9,
                "<<" | ">>" | ">>>" => 10,
                "+" | "-" => 11,
                "*" | "/" | "%" | "\\" => 12,
                "incontextof" => 13,
                _ => break,
            };
            if precedence < min {
                break;
            }
            if op == "*"
                && matches!(&self.tokens[self.pos + 1].kind, Kind::Symbol(s) if s == "," || s == ")")
            {
                self.pos += 1;
                left = Expr::Spread(Box::new(left));
                break;
            }
            self.pos += 1;
            let right = self.expression(if precedence == 1 || op == "incontextof" {
                precedence
            } else {
                precedence + 1
            })?;
            left = if precedence == 1 {
                Expr::Assign(Box::new(left), op, Box::new(right))
            } else {
                Expr::Binary(op, Box::new(left), Box::new(right))
            };
        }
        self.depth -= 1;
        Ok(left)
    }
    fn read(&mut self, e: Expr, optional: bool, at: &SourceLocation) -> Result<usize> {
        self.read_raw(e, optional, false, at)
    }
    fn read_raw(
        &mut self,
        e: Expr,
        optional: bool,
        raw: bool,
        at: &SourceLocation,
    ) -> Result<usize> {
        let out = self.reg();
        match e {
            Expr::Name(name) => {
                self.emit(
                    Op::Load {
                        out,
                        name,
                        optional,
                        raw,
                    },
                    at,
                );
            }
            Expr::Member(receiver, key) => {
                let object = self.compile_expr(*receiver, at)?;
                let key = self.compile_expr(*key, at)?;
                self.emit(
                    Op::Get {
                        out,
                        object,
                        key,
                        optional,
                        raw,
                    },
                    at,
                );
            }
            other => return self.compile_expr(other, at),
        }
        Ok(out)
    }
    fn compile_args(&mut self, args: Vec<Expr>, at: &SourceLocation) -> Result<Vec<Argument>> {
        args.into_iter()
            .map(|e| match e {
                Expr::ForwardArguments => Ok(Argument::Forward),
                Expr::Spread(e) => Ok(Argument::Spread(self.compile_expr(*e, at)?)),
                e => Ok(Argument::Value(self.compile_expr(e, at)?)),
            })
            .collect()
    }
    fn compile_expr(&mut self, e: Expr, at: &SourceLocation) -> Result<usize> {
        let out = self.reg();
        match e {
            Expr::ForwardArguments | Expr::Spread(_) => bail!("argument expansion outside call"),
            Expr::Value(value) => {
                self.emit(Op::Constant { out, value }, at);
            }
            Expr::With => {
                self.emit(Op::With { out }, at);
            }
            Expr::Name(_) | Expr::Member(..) => return self.read(e, false, at),
            Expr::Array(items) => {
                let items = items
                    .into_iter()
                    .map(|e| self.compile_expr(e, at))
                    .collect::<Result<_>>()?;
                self.emit(Op::Array { out, items }, at);
            }
            Expr::Dictionary(items) => {
                let items = items
                    .into_iter()
                    .map(|(k, v)| Ok((self.compile_expr(k, at)?, self.compile_expr(v, at)?)))
                    .collect::<Result<_>>()?;
                self.emit(Op::Dictionary { out, items }, at);
            }
            Expr::Function(function) => {
                self.emit(Op::Function { out, function }, at);
            }
            Expr::Assign(target, op, value) => {
                let target = self.target(*target, at)?;
                let old = if op != "=" {
                    Some(self.read_target(&target, at))
                } else {
                    None
                };
                let input = self.compile_expr(*value, at)?;
                if let Some(left) = old {
                    self.emit(
                        Op::Binary {
                            out,
                            op: op.trim_end_matches('=').into(),
                            left,
                            right: input,
                        },
                        at,
                    );
                } else {
                    self.emit(Op::Move { out, input }, at);
                }
                self.write_target(target, out, at);
            }
            Expr::Update(target, increment, prefix) => {
                let target = self.target(*target, at)?;
                let old = self.read_target(&target, at);
                let one = self.compile_expr(Expr::Value(Value::Integer(1)), at)?;
                let new = self.reg();
                self.emit(
                    Op::Binary {
                        out: new,
                        op: if increment { "+" } else { "-" }.into(),
                        left: old,
                        right: one,
                    },
                    at,
                );
                self.write_target(target, new, at);
                self.emit(
                    Op::Move {
                        out,
                        input: if prefix { new } else { old },
                    },
                    at,
                );
            }
            Expr::Unary(op, e) if op == "&" => return self.read_raw(*e, false, true, at),
            Expr::Unary(op, e) if op == "*" => {
                let input = self.compile_expr(*e, at)?;
                self.emit(Op::ReadProperty { out, input }, at);
            }
            Expr::Unary(op, e) => {
                if op == "typeof"
                    && let Expr::Member(receiver, key) = *e
                {
                    let object = self.compile_expr(*receiver, at)?;
                    let key = self.compile_expr(*key, at)?;
                    self.emit(Op::TypeOfMember { out, object, key }, at);
                    return Ok(out);
                }
                let input = self.compile_expr(*e, at)?;
                self.emit(Op::Unary { out, op, input }, at);
            }
            Expr::Binary(op, left, right) if op == "&&" || op == "||" => {
                let left = self.compile_expr(*left, at)?;
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
                let right = self.compile_expr(*right, at)?;
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
                let left = self.compile_expr(*left, at)?;
                let right = self.compile_expr(*right, at)?;
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
            Expr::Call(callee, args) => {
                let (callee, context) = if let Expr::Member(receiver, key) = *callee {
                    let object = self.compile_expr(*receiver, at)?;
                    let key = self.compile_expr(*key, at)?;
                    let callee = self.reg();
                    self.emit(
                        Op::Get {
                            out: callee,
                            object,
                            key,
                            optional: false,
                            raw: false,
                        },
                        at,
                    );
                    (callee, Some(object))
                } else {
                    (self.compile_expr(*callee, at)?, None)
                };
                let args = self.compile_args(args, at)?;
                self.emit(
                    Op::Call {
                        out,
                        callee,
                        context,
                        args,
                    },
                    at,
                );
            }
            Expr::Construct(callee, args) => {
                let callee = self.compile_expr(*callee, at)?;
                let args = self.compile_args(args, at)?;
                self.emit(Op::Construct { out, callee, args }, at);
            }
            Expr::Eval(e) => {
                let input = self.compile_expr(*e, at)?;
                self.emit(Op::Eval { out, input }, at);
            }
            Expr::Conditional(test, yes, no) => {
                let test = self.compile_expr(*test, at)?;
                let jump = self.emit(
                    Op::JumpUnless {
                        input: test,
                        target: 0,
                    },
                    at,
                );
                let input = self.compile_expr(*yes, at)?;
                self.emit(Op::Move { out, input }, at);
                let end = self.emit(Op::Jump { target: 0 }, at);
                self.patch(jump);
                let input = self.compile_expr(*no, at)?;
                self.emit(Op::Move { out, input }, at);
                self.patch(end);
            }
        }
        Ok(out)
    }
    fn target(&mut self, e: Expr, at: &SourceLocation) -> Result<Target> {
        match e {
            Expr::Name(name) => Ok(Target::Name(name, false)),
            Expr::Member(object, key) => Ok(Target::Member(
                self.compile_expr(*object, at)?,
                self.compile_expr(*key, at)?,
                false,
            )),
            Expr::Unary(op, value) if op == "&" => match self.target(*value, at)? {
                Target::Name(n, _) => Ok(Target::Name(n, true)),
                Target::Member(o, k, _) => Ok(Target::Member(o, k, true)),
                _ => bail!("invalid raw property assignment"),
            },
            Expr::Unary(op, value) if op == "*" => {
                Ok(Target::Property(self.compile_expr(*value, at)?))
            }
            _ => bail!("invalid assignment target"),
        }
    }
    fn read_target(&mut self, target: &Target, at: &SourceLocation) -> usize {
        let out = self.reg();
        self.emit(
            match target {
                Target::Name(name, raw) => Op::Load {
                    out,
                    name: name.clone(),
                    optional: false,
                    raw: *raw,
                },
                Target::Member(object, key, raw) => Op::Get {
                    out,
                    object: *object,
                    key: *key,
                    optional: false,
                    raw: *raw,
                },
                Target::Property(input) => Op::ReadProperty { out, input: *input },
            },
            at,
        );
        out
    }
    fn write_target(&mut self, target: Target, input: usize, at: &SourceLocation) {
        self.emit(
            match target {
                Target::Name(name, raw) => Op::Store { name, input, raw },
                Target::Member(object, key, raw) => Op::Set {
                    object,
                    key,
                    input,
                    raw,
                },
                Target::Property(property) => Op::WriteProperty { property, input },
            },
            at,
        );
    }
    fn end(&mut self) -> Result<()> {
        ensure!(
            self.eat(";") || self.is("}") || self.eof(),
            "expected semicolon; unsupported TJS statement continuation"
        );
        Ok(())
    }
    fn enter_scope(&mut self, at: &SourceLocation) {
        self.scopes += 1;
        self.emit(Op::EnterScope, at);
    }
    fn leave_scope(&mut self, at: &SourceLocation) {
        self.scopes -= 1;
        self.emit(Op::LeaveScope, at);
    }
    fn variables(&mut self, at: &SourceLocation) -> Result<()> {
        loop {
            let name = self.name()?;
            let e = if self.eat("=") {
                self.expression(0)?
            } else {
                Expr::Value(Value::Void)
            };
            let input = self.compile_expr(e, at)?;
            self.emit(Op::Declare { name, input }, at);
            if !self.eat(",") {
                break;
            }
        }
        Ok(())
    }
    fn begin_loop(&mut self) {
        self.loops.push(Loop {
            breaks: vec![],
            continues: vec![],
            scopes: self.scopes,
            handlers: self.handlers,
            withs: self.withs,
        });
    }
    fn finish_loop(&mut self, continue_at: usize) {
        let state = self.loops.pop().unwrap();
        for jump in state.breaks {
            self.patch(jump);
        }
        for jump in state.continues {
            self.patch_to(jump, continue_at);
        }
    }
    fn statement(&mut self) -> Result<()> {
        self.expr_nodes = 0;
        self.depth += 1;
        ensure!(self.depth <= 128, "TJS statement nesting exceeds limit");
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }
    fn property(&mut self) -> Result<Property> {
        let name = self.name()?;
        self.expect("{")?;
        let (mut getter, mut setter) = (None, None);
        while !self.eat("}") {
            if self.eat("getter") {
                ensure!(getter.is_none(), "duplicate getter");
                getter = Some(self.function(format!("{name}.getter"))?);
            } else if self.eat("setter") {
                ensure!(setter.is_none(), "duplicate setter");
                setter = Some(self.function(format!("{name}.setter"))?);
            } else {
                bail!("expected property getter or setter");
            }
        }
        ensure!(
            getter.as_ref().is_none_or(|f| f.parameters.is_empty()),
            "getter cannot take parameters"
        );
        ensure!(
            setter.as_ref().is_none_or(|f| f.parameters.len() == 1),
            "setter requires one parameter"
        );
        Ok(Property {
            name,
            getter,
            setter,
        })
    }
    fn class(&mut self, at: &SourceLocation) -> Result<()> {
        let name = self.name()?;
        let mut bases = vec![];
        if self.eat("extends") {
            loop {
                let base = self.expression(0)?;
                bases.push(self.compile_expr(base, at)?);
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect("{")?;
        let mut definition = Class {
            name: name.clone(),
            methods: vec![],
            properties: vec![],
            fields: vec![],
        };
        while !self.eat("}") {
            self.expr_nodes = 0;
            if self.eat(";") {
                continue;
            }
            if self.eat("function") {
                let name = self.name()?;
                definition.methods.push(self.function(name)?);
            } else if self.eat("property") {
                definition.properties.push(self.property()?);
            } else if self.eat("var") {
                loop {
                    let field = self.name()?;
                    let expr = if self.eat("=") {
                        self.expression(0)?
                    } else {
                        Expr::Value(Value::Void)
                    };
                    let initializer = Program {
                        storage: self.program.storage.clone(),
                        registers: 0,
                        code: vec![],
                    };
                    let parent = std::mem::replace(&mut self.program, initializer);
                    let input = self.compile_expr(expr, at)?;
                    self.emit(Op::Return { input }, at);
                    let initializer = std::mem::replace(&mut self.program, parent);
                    definition.fields.push((field, initializer));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.end()?;
            } else {
                return Err(unsupported(format!(
                    "unsupported class member {:?}",
                    self.tokens[self.pos].kind
                )));
            }
        }
        let out = self.reg();
        self.emit(
            Op::Class {
                out,
                definition: Box::new(definition),
                bases,
            },
            at,
        );
        self.emit(Op::Declare { name, input: out }, at);
        Ok(())
    }
    fn statement_inner(&mut self) -> Result<()> {
        let at = self.at();
        if self.eat(";") {
            return Ok(());
        }
        if self.eat("{") {
            self.enter_scope(&at);
            while !self.eat("}") {
                ensure!(!self.eof(), "unterminated TJS block");
                self.statement()?;
            }
            self.leave_scope(&at);
            return Ok(());
        }
        if self.eat("class") {
            return self.class(&at);
        }
        if self.eat("property") {
            let definition = self.property()?;
            let name = definition.name.clone();
            let out = self.reg();
            self.emit(
                Op::Property {
                    out,
                    definition: Box::new(definition),
                },
                &at,
            );
            self.emit(Op::Declare { name, input: out }, &at);
            return Ok(());
        }
        if self.eat("var") {
            self.variables(&at)?;
            return self.end();
        }
        if self.eat("function") {
            let name = self.name()?;
            let function = self.function(name.clone())?;
            let input = self.compile_expr(Expr::Function(Box::new(function)), &at)?;
            self.emit(Op::Declare { name, input }, &at);
            return Ok(());
        }
        if self.eat("try") {
            let exception = self.reg();
            let entry = self.emit(
                Op::Try {
                    target: 0,
                    exception,
                },
                &at,
            );
            self.handlers += 1;
            self.statement()?;
            self.handlers -= 1;
            self.emit(Op::EndTry, &at);
            let end = self.emit(Op::Jump { target: 0 }, &at);
            self.expect("catch")?;
            let name = if self.eat("(") {
                if self.eat(")") {
                    None
                } else {
                    let n = self.name()?;
                    self.expect(")")?;
                    Some(n)
                }
            } else {
                None
            };
            self.patch(entry);
            self.enter_scope(&at);
            if let Some(name) = name {
                self.emit(
                    Op::Declare {
                        name,
                        input: exception,
                    },
                    &at,
                );
            }
            self.statement()?;
            self.leave_scope(&at);
            self.patch(end);
            return Ok(());
        }
        if self.eat("throw") {
            let e = self.expression(0)?;
            let input = self.compile_expr(e, &at)?;
            self.emit(Op::Throw { input }, &at);
            return self.end();
        }
        if self.eat("with") {
            self.expect("(")?;
            let e = self.expression(0)?;
            self.expect(")")?;
            let input = self.compile_expr(e, &at)?;
            self.emit(Op::EnterWith { input }, &at);
            self.withs += 1;
            self.statement()?;
            self.withs -= 1;
            self.emit(Op::LeaveWith, &at);
            return Ok(());
        }
        if self.eat("if") {
            self.expect("(")?;
            let e = self.expression(0)?;
            self.expect(")")?;
            let input = self.compile_expr(e, &at)?;
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
        if self.eat("for") {
            self.enter_scope(&at);
            self.expect("(")?;
            if self.eat("var") {
                self.variables(&at)?;
            } else if !self.is(";") {
                let e = self.expression(0)?;
                self.compile_expr(e, &at)?;
            }
            self.expect(";")?;
            let top = self.program.code.len();
            let test = if self.is(";") {
                Expr::Value(Value::Integer(1))
            } else {
                self.expression(0)?
            };
            let input = self.compile_expr(test, &at)?;
            let end = self.emit(Op::JumpUnless { input, target: 0 }, &at);
            self.expect(";")?;
            let step = if self.is(")") {
                None
            } else {
                Some(self.expression(0)?)
            };
            self.expect(")")?;
            self.begin_loop();
            self.statement()?;
            let continue_at = self.program.code.len();
            if let Some(step) = step {
                self.compile_expr(step, &at)?;
            }
            self.emit(Op::Jump { target: top }, &at);
            self.patch(end);
            self.finish_loop(continue_at);
            self.leave_scope(&at);
            return Ok(());
        }
        if self.eat("while") {
            let top = self.program.code.len();
            self.expect("(")?;
            let e = self.expression(0)?;
            self.expect(")")?;
            let input = self.compile_expr(e, &at)?;
            let end = self.emit(Op::JumpUnless { input, target: 0 }, &at);
            self.begin_loop();
            self.statement()?;
            self.emit(Op::Jump { target: top }, &at);
            self.patch(end);
            self.finish_loop(top);
            return Ok(());
        }
        if self.is("break") || self.is("continue") {
            let is_break = self.eat("break");
            if !is_break {
                self.expect("continue")?;
            }
            let state = self
                .loops
                .last()
                .ok_or_else(|| anyhow::anyhow!("loop control outside a loop"))?;
            self.emit(
                Op::Unwind {
                    scopes: state.scopes,
                    handlers: state.handlers,
                    withs: state.withs,
                },
                &at,
            );
            let jump = self.emit(Op::Jump { target: 0 }, &at);
            let state = self.loops.last_mut().unwrap();
            if is_break {
                state.breaks.push(jump);
            } else {
                state.continues.push(jump);
            }
            return self.end();
        }
        if self.eat("return") {
            let e = if self.is(";") || self.is("}") || self.eof() {
                Expr::Value(Value::Void)
            } else {
                self.expression(0)?
            };
            let input = self.compile_expr(e, &at)?;
            self.emit(Op::Return { input }, &at);
            return self.end();
        }
        let e = self.expression(0)?;
        if self.eat("if") {
            self.expect("(")?;
            let test = self.expression(0)?;
            self.expect(")")?;
            let input = self.compile_expr(test, &at)?;
            let end = self.emit(Op::JumpUnless { input, target: 0 }, &at);
            self.compile_expr(e, &at)?;
            self.patch(end);
        } else {
            self.compile_expr(e, &at)?;
        }
        self.end()
    }
}
enum Target {
    Name(String, bool),
    Member(usize, usize, bool),
    Property(usize),
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
        scopes: 0,
        handlers: 0,
        withs: 0,
        loops: vec![],
    };
    let result = (|| -> Result<()> {
        if expression {
            let at = c.at();
            let e = c.expression(0)?;
            ensure!(c.eof(), "unexpected tokens after TJS expression");
            let input = c.compile_expr(e, &at)?;
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
        e.context(format!("{}:{}:{}", at.storage, at.line, at.column))
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
        for (source, expected) in [("1 || missing()", 1), ("0 ? missing() : 7", 7)] {
            assert_eq!(
                vm.execute(
                    &compile_expression("expr", source).unwrap(),
                    &mut (),
                    &mut 1000
                )
                .unwrap(),
                Value::Integer(expected)
            );
        }
    }
    #[test]
    fn bounded_errors_and_locations() {
        let p = compile("loop.tjs", "while(1) {}").unwrap();
        assert!(
            Vm::default()
                .execute(&p, &mut (), &mut 10)
                .unwrap_err()
                .to_string()
                .contains("loop.tjs:1:1")
        );
        assert!(
            format!("{:#}", compile("switch.tjs", "switch(x) {}").unwrap_err())
                .contains("unsupported TJS construct switch")
        );
    }
}
