use crate::{
    ast::*,
    lexer::{self, Kind, Token},
    Error, ErrorKind, JsResult, Value,
};
use std::collections::BTreeSet;
pub fn parse(source: &str, name: &str) -> JsResult<Vec<Stmt>> {
    let mut p = Parser {
        tokens: lexer::lex(source, name)?,
        i: 0,
        source,
        name,
        depth: 0,
        functions: 0,
        loops: 0,
    };
    let body = p.list(false)?;
    Ok(body)
}
struct Parser<'a> {
    tokens: Vec<Token>,
    i: usize,
    source: &'a str,
    name: &'a str,
    depth: usize,
    functions: usize,
    loops: usize,
}
impl Parser<'_> {
    fn t(&self) -> &Token {
        &self.tokens[self.i]
    }
    fn error(&self, msg: &str) -> Error {
        lexer::located(
            Error::new(
                if msg.contains("limit") {
                    ErrorKind::Limit
                } else {
                    ErrorKind::Syntax
                },
                msg,
            ),
            self.source,
            self.name,
            self.t().pos,
        )
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.t().is(s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> JsResult<()> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(self.error(&format!(
                "expected '{s}'; unsupported syntax is not executed"
            )))
        }
    }
    fn ident(&mut self) -> JsResult<String> {
        if let Kind::Word(w) = &self.t().kind {
            if ![
                "let",
                "const",
                "var",
                "with",
                "if",
                "else",
                "while",
                "for",
                "break",
                "continue",
                "return",
                "throw",
                "function",
                "this",
                "true",
                "false",
                "null",
                "class",
                "new",
                "import",
                "export",
                "async",
                "await",
                "yield",
                "delete",
                "typeof",
                "void",
                "in",
                "instanceof",
                "switch",
                "try",
                "catch",
                "finally",
                "do",
                "debugger",
                "super",
                "extends",
                "enum",
                "implements",
                "interface",
                "package",
                "private",
                "protected",
                "public",
                "static",
                "eval",
                "arguments",
            ]
            .contains(&w.as_str())
            {
                let w = w.clone();
                self.i += 1;
                return Ok(w);
            }
        }
        Err(self.error("expected supported binding identifier (reserved/unsupported keyword)"))
    }
    fn semi(&mut self) -> JsResult<()> {
        if self.eat(";")
            || self.t().is("}")
            || matches!(self.t().kind, Kind::End)
            || (self.i > 0 && self.t().line > self.tokens[self.i - 1].line)
        {
            Ok(())
        } else {
            Err(self.error("expected semicolon"))
        }
    }
    fn list(&mut self, block: bool) -> JsResult<Vec<Stmt>> {
        let mut body = Vec::new();
        let mut names = BTreeSet::new();
        while !matches!(self.t().kind, Kind::End) && !(block && self.t().is("}")) {
            let s = self.stmt(true)?;
            let bindings = match &s.kind {
                S::Decl(_, ds) => ds.iter().map(|(n, _)| n.clone()).collect(),
                S::Function(n, _) => vec![n.clone()],
                _ => Vec::new(),
            };
            for name in bindings {
                if !names.insert(name) {
                    return Err(self.error("duplicate lexical declaration"));
                }
            }
            body.push(s);
        }
        if block {
            self.expect("}")?;
        }
        Ok(body)
    }
    fn stmt(&mut self, lexical: bool) -> JsResult<Stmt> {
        self.depth += 1;
        if self.depth > 96 {
            return Err(self.error("parser nesting limit"));
        }
        let r = self.stmt_inner(lexical);
        self.depth -= 1;
        r
    }
    fn stmt_inner(&mut self, lexical: bool) -> JsResult<Stmt> {
        let pos = self.t().pos;
        let kind = if self.eat(";") {
            S::Empty
        } else if self.eat("{") {
            S::Block(self.list(true)?)
        } else if self.t().is("let") || self.t().is("const") {
            if !lexical {
                return Err(self.error("lexical declaration requires a block"));
            }
            let constant = self.eat("const");
            if !constant {
                self.expect("let")?;
            }
            let ds = self.declarations(constant)?;
            self.semi()?;
            S::Decl(constant, ds)
        } else if self.eat("function") {
            if !lexical {
                return Err(
                    self.error("function declaration requires a block; Annex B is unsupported")
                );
            }
            let name = self.ident()?;
            let (params, body) = self.function_tail()?;
            S::Function(
                name,
                Expr {
                    pos,
                    kind: E::Function(None, params, body, FunctionKind::Ordinary),
                },
            )
        } else if self.eat("if") {
            self.expect("(")?;
            let c = self.expr(0)?;
            self.expect(")")?;
            let yes = Box::new(self.stmt(false)?);
            let no = if self.eat("else") {
                Some(Box::new(self.stmt(false)?))
            } else {
                None
            };
            S::If(c, yes, no)
        } else if self.eat("while") {
            self.expect("(")?;
            let c = self.expr(0)?;
            self.expect(")")?;
            self.loops += 1;
            let b = self.stmt(false)?;
            self.loops -= 1;
            S::While(c, Box::new(b))
        } else if self.eat("for") {
            self.expect("(")?;
            let init = if self.eat(";") {
                None
            } else {
                let p = self.t().pos;
                let k = if self.t().is("let") || self.t().is("const") {
                    let constant = self.eat("const");
                    if !constant {
                        self.expect("let")?;
                    }
                    S::Decl(constant, self.declarations(constant)?)
                } else {
                    S::Expr(self.expr(0)?)
                };
                self.expect(";")?;
                Some(Box::new(Stmt { kind: k, pos: p }))
            };
            let test = if self.eat(";") {
                None
            } else {
                let e = self.expr(0)?;
                self.expect(";")?;
                Some(e)
            };
            let step = if self.eat(")") {
                None
            } else {
                let e = self.expr(0)?;
                self.expect(")")?;
                Some(e)
            };
            self.loops += 1;
            let b = self.stmt(false)?;
            self.loops -= 1;
            S::For(init, test, step, Box::new(b))
        } else if self.eat("return") {
            if self.functions == 0 {
                return Err(self.error("return outside function"));
            }
            let e = if self.t().is(";")
                || self.t().is("}")
                || matches!(self.t().kind, Kind::End)
                || self.t().line > self.tokens[self.i - 1].line
            {
                None
            } else {
                Some(self.expr(0)?)
            };
            self.semi()?;
            S::Return(e)
        } else if self.t().is("break") || self.t().is("continue") {
            let br = self.eat("break");
            if !br {
                self.expect("continue")?;
            }
            if self.loops == 0 {
                return Err(self.error("break/continue outside loop"));
            }
            self.semi()?;
            if br {
                S::Break
            } else {
                S::Continue
            }
        } else if self.eat("throw") {
            if self.t().line > self.tokens[self.i - 1].line {
                return Err(self.error("newline after throw"));
            }
            let e = self.expr(0)?;
            self.semi()?;
            S::Throw(e)
        } else {
            let e = self.expr(0)?;
            self.semi()?;
            S::Expr(e)
        };
        Ok(Stmt { kind, pos })
    }
    fn declarations(&mut self, constant: bool) -> JsResult<Vec<(String, Option<Expr>)>> {
        let mut ds = Vec::new();
        let mut names = BTreeSet::new();
        loop {
            let n = self.ident()?;
            if !names.insert(n.clone()) {
                return Err(self.error("duplicate binding"));
            }
            let e = if self.eat("=") {
                Some(self.expr(0)?)
            } else {
                None
            };
            if constant && e.is_none() {
                return Err(self.error("const requires initializer"));
            }
            ds.push((n, e));
            if !self.eat(",") {
                break;
            }
        }
        Ok(ds)
    }
    fn params(&mut self) -> JsResult<Vec<String>> {
        self.expect("(")?;
        let mut ps = Vec::new();
        if !self.eat(")") {
            loop {
                let n = self.ident()?;
                if ps.contains(&n) {
                    return Err(self.error("duplicate strict parameter"));
                }
                ps.push(n);
                if self.eat(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        Ok(ps)
    }
    fn function_body(&mut self, ps: &[String]) -> JsResult<Vec<Stmt>> {
        self.expect("{")?;
        let loops = self.loops;
        self.loops = 0;
        self.functions += 1;
        let body = self.list(true)?;
        self.functions -= 1;
        self.loops = loops;
        for s in &body {
            let names = match &s.kind {
                S::Decl(_, ds) => ds.iter().map(|(n, _)| n).collect(),
                S::Function(n, _) => vec![n],
                _ => Vec::new(),
            };
            if names.iter().any(|n| ps.contains(n)) {
                return Err(self.error("parameter conflicts with lexical declaration"));
            }
        }
        Ok(body)
    }
    fn function_tail(&mut self) -> JsResult<(Vec<String>, Vec<Stmt>)> {
        let ps = self.params()?;
        let b = self.function_body(&ps)?;
        Ok((ps, b))
    }
    fn expr(&mut self, min: u8) -> JsResult<Expr> {
        self.depth += 1;
        if self.depth > 96 {
            return Err(self.error("parser nesting limit"));
        }
        let r = self.expr_inner(min);
        self.depth -= 1;
        r
    }
    fn expr_inner(&mut self, min: u8) -> JsResult<Expr> {
        let pos = self.t().pos;
        let mut left = self.prefix()?;
        let mut chain = 0;
        loop {
            self.check_height(&left)?;
            chain += 1;
            if chain > 64 {
                return Err(self.error("expression chain limit"));
            }
            if min <= 15 && self.eat(".") {
                let key = if let Kind::Word(w) = &self.t().kind {
                    let w = w.clone();
                    self.i += 1;
                    w
                } else {
                    return Err(self.error("expected property name"));
                };
                left = Expr {
                    pos,
                    kind: E::Member(
                        Box::new(left),
                        Box::new(Expr {
                            pos,
                            kind: E::Literal(Value::string(&key)),
                        }),
                    ),
                };
                continue;
            }
            if min <= 15 && self.eat("[") {
                let key = self.expr(0)?;
                self.expect("]")?;
                left = Expr {
                    pos,
                    kind: E::Member(Box::new(left), Box::new(key)),
                };
                continue;
            }
            if min <= 15 && self.eat("(") {
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.expr(0)?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                left = Expr {
                    pos,
                    kind: E::Call(Box::new(left), args),
                };
                continue;
            }
            if min <= 2 && self.eat("?") {
                let yes = self.expr(0)?;
                self.expect(":")?;
                let no = self.expr(1)?;
                left = Expr {
                    pos,
                    kind: E::Conditional(Box::new(left), Box::new(yes), Box::new(no)),
                };
                continue;
            }
            let (op, prec) = match &self.t().kind {
                Kind::Punct(op) => (
                    op.clone(),
                    match op.as_str() {
                        "=" => 1,
                        "??" | "||" => 3,
                        "&&" => 4,
                        "===" | "!==" => 8,
                        "<" | ">" | "<=" | ">=" => 9,
                        "+" | "-" => 11,
                        "*" | "/" | "%" => 12,
                        _ => 0,
                    },
                ),
                _ => break,
            };
            if prec == 0 || prec < min {
                break;
            }
            self.i += 1;
            let right = self.expr(if op == "=" { prec } else { prec + 1 })?;
            if op == "=" {
                if !Self::target(&left) {
                    return Err(self.error("invalid assignment target"));
                }
                left = Expr {
                    pos,
                    kind: E::Assign(Box::new(left), Box::new(right)),
                };
            } else {
                for e in [&left, &right] {
                    if let E::Binary(other, _, _) = &e.kind {
                        if (op == "??" && (other == "&&" || other == "||"))
                            || ((op == "&&" || op == "||") && other == "??")
                        {
                            return Err(
                                self.error("parentheses required when mixing ?? with &&/||")
                            );
                        }
                    }
                }
                left = Expr {
                    pos,
                    kind: E::Binary(op, Box::new(left), Box::new(right)),
                };
            }
        }
        Ok(left)
    }
    fn check_height(&self, root: &Expr) -> JsResult<()> {
        let mut pending = vec![(root, 1)];
        while let Some((e, d)) = pending.pop() {
            if d > 96 {
                return Err(self.error("expression nesting limit"));
            }
            let mut add = |x| pending.push((x, d + 1));
            match &e.kind {
                E::Group(x) | E::Unary(_, x) => add(x.as_ref()),
                E::Binary(_, a, b) | E::Assign(a, b) | E::Member(a, b) => {
                    add(a.as_ref());
                    add(b.as_ref());
                }
                E::Conditional(a, b, c) => {
                    add(a.as_ref());
                    add(b.as_ref());
                    add(c.as_ref());
                }
                E::Call(f, args) => {
                    add(f.as_ref());
                    for a in args {
                        add(a);
                    }
                }
                E::Object(ps) => {
                    for (_, x) in ps {
                        add(x);
                    }
                }
                E::Array(xs) => {
                    for x in xs {
                        add(x);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn target(e: &Expr) -> bool {
        match &e.kind {
            E::Id(_) | E::Member(_, _) => true,
            E::Group(e) => Self::target(e),
            _ => false,
        }
    }
    fn prefix(&mut self) -> JsResult<Expr> {
        let pos = self.t().pos;
        // Cover grammar restricted to plain identifier parameter lists.
        let arrow = if self.t().is("(") {
            let mut j = self.i + 1;
            while j < self.tokens.len()
                && (matches!(self.tokens[j].kind, Kind::Word(_)) || self.tokens[j].is(","))
            {
                j += 1;
            }
            self.tokens.get(j).is_some_and(|t| t.is(")"))
                && self.tokens.get(j + 1).is_some_and(|t| t.is("=>"))
        } else {
            matches!(self.t().kind, Kind::Word(_))
                && self.tokens.get(self.i + 1).is_some_and(|t| t.is("=>"))
        };
        let kind = if arrow {
            let ps = if self.t().is("(") {
                self.params()?
            } else {
                vec![self.ident()?]
            };
            if self.t().line > self.tokens[self.i - 1].line {
                return Err(self.error("newline before arrow"));
            }
            self.expect("=>")?;
            let body = if self.t().is("{") {
                self.function_body(&ps)?
            } else {
                vec![Stmt {
                    pos,
                    kind: S::Return(Some(self.expr(1)?)),
                }]
            };
            E::Function(None, ps, body, FunctionKind::Arrow)
        } else if let Some(v) = self.t().literal() {
            self.i += 1;
            E::Literal(v)
        } else if self.eat("true") {
            E::Literal(Value::Boolean(true))
        } else if self.eat("false") {
            E::Literal(Value::Boolean(false))
        } else if self.eat("null") {
            E::Literal(Value::Null)
        } else if self.eat("this") {
            E::This
        } else if self.t().is("!") || self.t().is("-") || self.t().is("+") {
            let op = match &self.t().kind {
                Kind::Punct(s) => s.clone(),
                _ => unreachable!(),
            };
            self.i += 1;
            E::Unary(op, Box::new(self.expr(13)?))
        } else if self.eat("(") {
            let e = self.expr(0)?;
            self.expect(")")?;
            E::Group(Box::new(e))
        } else if self.eat("function") {
            let name = if self.t().is("(") {
                None
            } else {
                Some(self.ident()?)
            };
            let (ps, b) = self.function_tail()?;
            E::Function(name, ps, b, FunctionKind::Ordinary)
        } else if self.eat("[") {
            let mut es = Vec::new();
            if !self.eat("]") {
                loop {
                    es.push(self.expr(0)?);
                    if self.eat("]") {
                        break;
                    }
                    self.expect(",")?;
                    if self.eat("]") {
                        break;
                    }
                }
            }
            E::Array(es)
        } else if self.eat("{") {
            let mut props = Vec::new();
            if !self.eat("}") {
                loop {
                    let key = match &self.t().kind {
                        Kind::Word(w) => w.clone(),
                        Kind::String(s) => String::from_utf16(s)
                            .map_err(|_| self.error("surrogate property names unsupported"))?,
                        Kind::Number(n) => Value::Number(*n).display(),
                        _ => return Err(self.error("unsupported object property")),
                    };
                    self.i += 1;
                    if key == "__proto__" {
                        return Err(self.error("__proto__ literal operations unsupported"));
                    }
                    let e = if self.eat(":") {
                        self.expr(0)?
                    } else if self.t().is("(") {
                        let (ps, b) = self.function_tail()?;
                        Expr {
                            pos,
                            kind: E::Function(None, ps, b, FunctionKind::Method),
                        }
                    } else {
                        return Err(self.error(
                            "object shorthand/accessors unsupported; expected ':' or method",
                        ));
                    };
                    props.push((key, e));
                    if self.eat("}") {
                        break;
                    }
                    self.expect(",")?;
                    if self.eat("}") {
                        break;
                    }
                }
            }
            E::Object(props)
        } else {
            E::Id(self.ident()?)
        };
        Ok(Expr { kind, pos })
    }
}
