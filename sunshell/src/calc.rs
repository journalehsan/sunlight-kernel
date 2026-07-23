//! Small expression engine for the shell's `=` builtin.
//!
//! This module intentionally keeps parsing/evaluation independent from
//! persistence. The only KV-aware code is the `history` wrapper near the end.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const EPS: f64 = 1.0e-12;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Equal,
    Comma,
    LeftParen,
    RightParen,
    End,
}

#[derive(Clone, Debug)]
struct SpannedToken {
    token: Token,
    text: String,
}

struct Lexer<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn next(&mut self) -> Result<SpannedToken, CalcError> {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        let start = self.pos;
        if self.pos >= self.bytes.len() {
            return Ok(SpannedToken {
                token: Token::End,
                text: String::new(),
            });
        }

        let ch = self.bytes[self.pos];
        let token = match ch {
            b'0'..=b'9' | b'.' => self.lex_number()?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident(),
            b'+' => {
                self.pos += 1;
                Token::Plus
            }
            b'-' => {
                self.pos += 1;
                Token::Minus
            }
            b'*' => {
                self.pos += 1;
                Token::Star
            }
            b'/' => {
                self.pos += 1;
                Token::Slash
            }
            b'%' => {
                self.pos += 1;
                Token::Percent
            }
            b'^' => {
                self.pos += 1;
                Token::Caret
            }
            b'=' => {
                self.pos += 1;
                Token::Equal
            }
            b',' => {
                self.pos += 1;
                Token::Comma
            }
            b'(' => {
                self.pos += 1;
                Token::LeftParen
            }
            b')' => {
                self.pos += 1;
                Token::RightParen
            }
            _ => return Err(CalcError::new("invalid character")),
        };

        Ok(SpannedToken {
            token,
            text: self.input[start..self.pos].to_string(),
        })
    }

    fn lex_number(&mut self) -> Result<Token, CalcError> {
        let start = self.pos;
        let mut saw_digit = false;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            saw_digit = true;
            self.pos += 1;
        }
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                saw_digit = true;
                self.pos += 1;
            }
        }
        if !saw_digit {
            return Err(CalcError::new("expected digit after '.'"));
        }
        let s = &self.input[start..self.pos];
        let n = s
            .parse::<f64>()
            .map_err(|_| CalcError::new("number too large"))?;
        Ok(Token::Number(n))
    }

    fn lex_ident(&mut self) -> Token {
        let start = self.pos;
        while self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_alphanumeric() || self.bytes[self.pos] == b'_')
        {
            self.pos += 1;
        }
        Token::Ident(self.input[start..self.pos].to_string())
    }
}

#[derive(Clone, Debug)]
enum Expr {
    Number(f64),
    Var(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Func {
        name: String,
        arg: Box<Expr>,
    },
    Group(Box<Expr>),
}

#[derive(Clone, Copy, Debug)]
enum UnaryOp {
    Plus,
    Minus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    current: SpannedToken,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Result<Self, CalcError> {
        if input.trim().is_empty() {
            return Err(CalcError::new("empty expression"));
        }
        let mut lexer = Lexer::new(input);
        let current = lexer.next()?;
        Ok(Self { lexer, current })
    }

    fn advance(&mut self) -> Result<(), CalcError> {
        self.current = self.lexer.next()?;
        Ok(())
    }

    fn parse_expression(&mut self) -> Result<Expr, CalcError> {
        let expr = self.parse_add_sub()?;
        if self.current.token != Token::End {
            return Err(CalcError::new("unexpected token"));
        }
        Ok(expr)
    }

    fn parse_add_sub(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.parse_mul_div()?;
        loop {
            let op = match self.current.token {
                Token::Plus => BinaryOp::Add,
                Token::Minus => BinaryOp::Sub,
                _ => break,
            };
            let op_text = self.current.text.clone();
            self.advance()?;
            if matches!(self.current.token, Token::End | Token::RightParen) {
                return Err(CalcError::new(format!(
                    "expected expression after '{}'",
                    op_text
                )));
            }
            let right = self.parse_mul_div()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_mul_div(&mut self) -> Result<Expr, CalcError> {
        let mut left = self.parse_unary()?;
        loop {
            let explicit = match self.current.token {
                Token::Star => Some(BinaryOp::Mul),
                Token::Slash => Some(BinaryOp::Div),
                Token::Percent => Some(BinaryOp::Mod),
                _ => None,
            };

            let op = if let Some(op) = explicit {
                let op_text = self.current.text.clone();
                self.advance()?;
                if matches!(self.current.token, Token::End | Token::RightParen) {
                    return Err(CalcError::new(format!(
                        "expected expression after '{}'",
                        op_text
                    )));
                }
                op
            } else if self.starts_implicit_factor() {
                // Implicit multiplication is inserted as `*` at the same
                // precedence and left associativity as explicit multiplication.
                BinaryOp::Mul
            } else {
                break;
            };

            let right = self.parse_unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn starts_implicit_factor(&self) -> bool {
        matches!(
            self.current.token,
            Token::Number(_) | Token::Ident(_) | Token::LeftParen
        )
    }

    fn parse_unary(&mut self) -> Result<Expr, CalcError> {
        match self.current.token {
            Token::Plus => {
                self.advance()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Plus,
                    expr: Box::new(self.parse_unary()?),
                })
            }
            Token::Minus => {
                self.advance()?;
                // Unary signs intentionally bind looser than power because the
                // operand is parsed through `parse_unary -> parse_power`:
                // `-2^2` becomes `-(2^2)`, while `(-2)^2` stays grouped.
                Ok(Expr::Unary {
                    op: UnaryOp::Minus,
                    expr: Box::new(self.parse_unary()?),
                })
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<Expr, CalcError> {
        let left = self.parse_primary()?;
        if self.current.token == Token::Caret {
            self.advance()?;
            let right = self.parse_unary()?;
            return Ok(Expr::Binary {
                op: BinaryOp::Pow,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn parse_primary(&mut self) -> Result<Expr, CalcError> {
        match self.current.token.clone() {
            Token::Number(n) => {
                self.advance()?;
                Ok(Expr::Number(n))
            }
            Token::Ident(name) => {
                self.advance()?;
                if is_function_name(&name) && self.current.token == Token::LeftParen {
                    self.advance()?;
                    let arg = self.parse_add_sub()?;
                    if self.current.token != Token::RightParen {
                        return Err(CalcError::new("unmatched parenthesis"));
                    }
                    self.advance()?;
                    Ok(Expr::Func {
                        name,
                        arg: Box::new(arg),
                    })
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Token::LeftParen => {
                self.advance()?;
                let expr = self.parse_add_sub()?;
                if self.current.token != Token::RightParen {
                    return Err(CalcError::new("unmatched parenthesis"));
                }
                self.advance()?;
                Ok(Expr::Group(Box::new(expr)))
            }
            _ => Err(CalcError::new("expected expression")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalcError {
    message: String,
}

impl CalcError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl core::fmt::Display for CalcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

pub struct CalcSession {
    vars: BTreeMap<String, f64>,
}

impl CalcSession {
    pub fn new() -> Self {
        Self {
            vars: BTreeMap::new(),
        }
    }

    pub fn run_command(&mut self, input: &str) -> String {
        let input = input.trim();
        if input.is_empty() {
            return "calc error: empty expression\n".to_string();
        }

        let result = if let Some(rest) = input.strip_prefix("explain ") {
            self.explain(rest.trim())
        } else if let Some(rest) = input.strip_prefix("solve ") {
            self.solve(rest.trim())
        } else if input == "history" || input.starts_with("history ") {
            history::handle_history_command(input)
        } else if input.starts_with("file ") {
            Err(CalcError::new("file mode requires shell VFS access"))
        } else if let Some((name, expr)) = assignment(input) {
            self.assign(name, expr)
        } else {
            self.eval_to_string(input)
        };

        match result {
            Ok(out) => out,
            Err(e) => format!("calc error: {}\n", e),
        }
    }

    #[cfg_attr(not(feature = "sunlight"), allow(dead_code))]
    pub fn run_file_contents(&mut self, input: &str, original: &str) -> String {
        let mut expr = String::new();
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            if !expr.is_empty() {
                expr.push(' ');
            }
            expr.push_str(trimmed);
        }
        if expr.is_empty() {
            return "calc error: empty expression\n".to_string();
        }
        self.eval_success(&expr, original)
    }

    fn assign(&mut self, name: &str, expr: &str) -> Result<String, CalcError> {
        let parsed = parse(expr)?;
        let mut used = Vec::new();
        let value = eval(&parsed, &self.vars, &mut used)?;
        self.vars.insert(name.to_string(), value);
        let out = format!("{} = {}\n", name, format_number(value));
        self.save_history(
            expr,
            expr,
            &format!("{} = {}", name, format_number(value)),
            &used,
        );
        Ok(out)
    }

    fn eval_to_string(&mut self, input: &str) -> Result<String, CalcError> {
        Ok(self.eval_success(input, input))
    }

    fn eval_success(&mut self, input: &str, history_input: &str) -> String {
        match parse(input).and_then(|expr| {
            let mut used = Vec::new();
            let value = eval(&expr, &self.vars, &mut used)?;
            Ok((value, used))
        }) {
            Ok((value, used)) => {
                let result = format_number(value);
                let out = format!("{}\n", result);
                // History is best-effort and uses bounded IPC. Compute and
                // return the user-visible result before/around the side-effect.
                self.save_history(history_input, input, &result, &used);
                out
            }
            Err(e) => format!("calc error: {}\n", e),
        }
    }

    fn explain(&mut self, input: &str) -> Result<String, CalcError> {
        let expr = parse(input)?;
        let mut used = Vec::new();
        let mut out = String::new();
        let mut idx = 1usize;
        collect_group_explain(&expr, &self.vars, &mut used, &mut idx, &mut out)?;
        let value = eval(&expr, &self.vars, &mut used)?;
        let result = format_number(value);
        out.push_str("result = ");
        out.push_str(&result);
        out.push('\n');
        self.save_history(input, input, &result, &used);
        Ok(out)
    }

    fn solve(&mut self, input: &str) -> Result<String, CalcError> {
        let out = solve_linear(input)?;
        if !out.starts_with("calc error:") {
            self.save_history(input, input, out.trim(), &[]);
        }
        Ok(out)
    }

    fn save_history(&self, input: &str, normalized: &str, result: &str, vars: &[String]) {
        // Result is prepared before history save. History is best-effort and
        // bounded (via ipc_call_timeout). A slow/missing KV cannot prevent the
        // calculation result from being returned to the user.
        let record = history::HistoryRecord::new(input, normalized, result, vars);
        if let Err(e) = history::calc_history_put(&record) {
            history::warn(&format!("calc history unavailable: {}", e));
        }
    }
}

fn parse(input: &str) -> Result<Expr, CalcError> {
    Parser::new(input)?.parse_expression()
}

fn assignment(input: &str) -> Option<(&str, &str)> {
    let idx = input.find('=')?;
    let (left, right) = input.split_at(idx);
    let name = left.trim();
    if is_ident(name) {
        Some((name, right[1..].trim()))
    } else {
        None
    }
}

fn is_ident(s: &str) -> bool {
    let mut bytes = s.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn eval(
    expr: &Expr,
    vars: &BTreeMap<String, f64>,
    used: &mut Vec<String>,
) -> Result<f64, CalcError> {
    let value = match expr {
        Expr::Number(n) => *n,
        Expr::Var(name) if name == "pi" => core::f64::consts::PI,
        Expr::Var(name) if name == "e" => core::f64::consts::E,
        Expr::Var(name) => {
            if !used.iter().any(|v| v == name) {
                used.push(name.clone());
            }
            *vars
                .get(name)
                .ok_or_else(|| CalcError::new(format!("unknown variable '{}'", name)))?
        }
        Expr::Unary { op, expr } => {
            let v = eval(expr, vars, used)?;
            match op {
                UnaryOp::Plus => v,
                UnaryOp::Minus => -v,
            }
        }
        Expr::Binary { op, left, right } => {
            let l = eval(left, vars, used)?;
            let r = eval(right, vars, used)?;
            match op {
                BinaryOp::Add => l + r,
                BinaryOp::Sub => l - r,
                BinaryOp::Mul => l * r,
                BinaryOp::Div => {
                    if near_zero(r) {
                        return Err(CalcError::new("division by zero"));
                    }
                    l / r
                }
                BinaryOp::Mod => {
                    if near_zero(r) {
                        return Err(CalcError::new("division by zero"));
                    }
                    l % r
                }
                BinaryOp::Pow => libm::pow(l, r),
            }
        }
        Expr::Func { name, arg } => {
            let v = eval(arg, vars, used)?;
            eval_func(name, v)?
        }
        Expr::Group(inner) => eval(inner, vars, used)?,
    };

    if value.is_finite() {
        Ok(value)
    } else {
        Err(CalcError::new("non-finite result"))
    }
}

fn eval_func(name: &str, v: f64) -> Result<f64, CalcError> {
    match name {
        "sqrt" => {
            if v < 0.0 {
                Err(CalcError::new("sqrt domain error"))
            } else {
                Ok(libm::sqrt(v))
            }
        }
        "abs" => Ok(libm::fabs(v)),
        "sin" => Ok(libm::sin(v)),
        "cos" => Ok(libm::cos(v)),
        "tan" => Ok(libm::tan(v)),
        "ln" => {
            if v <= 0.0 {
                Err(CalcError::new("ln domain error"))
            } else {
                Ok(libm::log(v))
            }
        }
        "log" => {
            if v <= 0.0 {
                Err(CalcError::new("log domain error"))
            } else {
                Ok(libm::log10(v))
            }
        }
        "exp" => Ok(libm::exp(v)),
        _ => Err(CalcError::new(format!("unknown function '{}'", name))),
    }
}

fn is_function_name(name: &str) -> bool {
    matches!(
        name,
        "sqrt" | "abs" | "sin" | "cos" | "tan" | "ln" | "log" | "exp"
    )
}

fn collect_group_explain(
    expr: &Expr,
    vars: &BTreeMap<String, f64>,
    used: &mut Vec<String>,
    idx: &mut usize,
    out: &mut String,
) -> Result<(), CalcError> {
    match expr {
        Expr::Group(inner) => {
            collect_group_explain(inner, vars, used, idx, out)?;
            if matches!(**inner, Expr::Binary { .. }) {
                let value = eval(inner, vars, used)?;
                out.push_str(&format!(
                    "a{} = {} = {}\n",
                    *idx,
                    expr_to_string(inner),
                    format_number(value)
                ));
                *idx += 1;
            }
        }
        Expr::Unary { expr, .. } | Expr::Func { arg: expr, .. } => {
            collect_group_explain(expr, vars, used, idx, out)?;
        }
        Expr::Binary { left, right, .. } => {
            collect_group_explain(left, vars, used, idx, out)?;
            collect_group_explain(right, vars, used, idx, out)?;
        }
        Expr::Number(_) | Expr::Var(_) => {}
    }
    Ok(())
}

fn solve_linear(input: &str) -> Result<String, CalcError> {
    let (equation, target) = split_for_target(input);
    let eq_pos = find_top_level_equal(equation)
        .ok_or_else(|| CalcError::new("solve expects an equation with '='"))?;
    let left = parse(equation[..eq_pos].trim())?;
    let right = parse(equation[eq_pos + 1..].trim())?;
    let mut form = to_linear(&left)?;
    form.sub(&to_linear(&right)?);

    let target_name = match target {
        Some(t) => {
            if !is_ident(t) {
                return Err(CalcError::new("invalid solve target"));
            }
            t.to_string()
        }
        None if form.coeffs.len() == 1 => form.coeffs.keys().next().unwrap().clone(),
        None => {
            return Ok("calc error: equation has multiple variables; use 'for <var>'\n".to_string())
        }
    };

    let coeff = *form
        .coeffs
        .get(&target_name)
        .ok_or_else(|| CalcError::new(format!("unknown solve variable '{}'", target_name)))?;
    if near_zero(coeff) {
        return Err(CalcError::new("variable coefficient is zero"));
    }

    if form.coeffs.len() == 1 {
        let value = -form.constant / coeff;
        return Ok(format!("{} = {}\n", target_name, format_number(value)));
    }

    let symbolic = isolate_symbolic(&form, &target_name, coeff);
    Ok(format!("{} = {}\n", target_name, symbolic))
}

fn split_for_target(input: &str) -> (&str, Option<&str>) {
    if let Some(idx) = input.rfind(" for ") {
        (&input[..idx], Some(input[idx + 5..].trim()))
    } else {
        (input, None)
    }
}

fn find_top_level_equal(input: &str) -> Option<usize> {
    let mut depth = 0isize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            '=' if depth == 0 => return Some(idx),
            _ => {}
        }
    }
    None
}

#[derive(Clone, Debug)]
struct LinearForm {
    constant: f64,
    coeffs: BTreeMap<String, f64>,
}

impl LinearForm {
    fn constant(value: f64) -> Self {
        Self {
            constant: value,
            coeffs: BTreeMap::new(),
        }
    }

    fn var(name: &str) -> Self {
        let mut coeffs = BTreeMap::new();
        coeffs.insert(name.to_string(), 1.0);
        Self {
            constant: 0.0,
            coeffs,
        }
    }

    fn add(&mut self, other: &Self) {
        self.constant += other.constant;
        for (name, coeff) in &other.coeffs {
            *self.coeffs.entry(name.clone()).or_insert(0.0) += coeff;
        }
        self.prune();
    }

    fn sub(&mut self, other: &Self) {
        self.constant -= other.constant;
        for (name, coeff) in &other.coeffs {
            *self.coeffs.entry(name.clone()).or_insert(0.0) -= coeff;
        }
        self.prune();
    }

    fn scale(&mut self, factor: f64) {
        self.constant *= factor;
        for coeff in self.coeffs.values_mut() {
            *coeff *= factor;
        }
        self.prune();
    }

    fn is_constant(&self) -> bool {
        self.coeffs.is_empty()
    }

    fn prune(&mut self) {
        let zeros: Vec<String> = self
            .coeffs
            .iter()
            .filter_map(|(name, coeff)| near_zero(*coeff).then(|| name.clone()))
            .collect();
        for name in zeros {
            self.coeffs.remove(&name);
        }
    }
}

fn to_linear(expr: &Expr) -> Result<LinearForm, CalcError> {
    // Solver intentionally reduces only linear expressions of the form
    // constant + sum(coeff[var] * var). Products of variables, variable
    // denominators, variable powers, and functions over variables are rejected;
    // this is not a CAS.
    match expr {
        Expr::Number(n) => Ok(LinearForm::constant(*n)),
        Expr::Var(name) if name == "pi" => Ok(LinearForm::constant(core::f64::consts::PI)),
        Expr::Var(name) if name == "e" => Ok(LinearForm::constant(core::f64::consts::E)),
        Expr::Var(name) => Ok(LinearForm::var(name)),
        Expr::Group(inner) => to_linear(inner),
        Expr::Unary { op, expr } => {
            let mut form = to_linear(expr)?;
            if matches!(op, UnaryOp::Minus) {
                form.scale(-1.0);
            }
            Ok(form)
        }
        Expr::Binary { op, left, right } => {
            let mut l = to_linear(left)?;
            let r = to_linear(right)?;
            match op {
                BinaryOp::Add => {
                    l.add(&r);
                    Ok(l)
                }
                BinaryOp::Sub => {
                    l.sub(&r);
                    Ok(l)
                }
                BinaryOp::Mul => {
                    if l.is_constant() {
                        let mut out = r;
                        out.scale(l.constant);
                        Ok(out)
                    } else if r.is_constant() {
                        l.scale(r.constant);
                        Ok(l)
                    } else {
                        Err(CalcError::new("non-linear term in equation"))
                    }
                }
                BinaryOp::Div => {
                    if !r.is_constant() {
                        return Err(CalcError::new("variable in denominator"));
                    }
                    if near_zero(r.constant) {
                        return Err(CalcError::new("division by zero"));
                    }
                    l.scale(1.0 / r.constant);
                    Ok(l)
                }
                BinaryOp::Mod | BinaryOp::Pow => {
                    if l.is_constant() && r.is_constant() {
                        let vars = BTreeMap::new();
                        let mut used = Vec::new();
                        Ok(LinearForm::constant(eval(expr, &vars, &mut used)?))
                    } else {
                        Err(CalcError::new("non-linear term in equation"))
                    }
                }
            }
        }
        Expr::Func { .. } => {
            let form = to_linear_func_const(expr)?;
            Ok(form)
        }
    }
}

fn to_linear_func_const(expr: &Expr) -> Result<LinearForm, CalcError> {
    let form = match expr {
        Expr::Func { arg, .. } => to_linear(arg)?,
        _ => return Err(CalcError::new("non-linear function in equation")),
    };
    if !form.is_constant() {
        return Err(CalcError::new("non-linear function in equation"));
    }
    let vars = BTreeMap::new();
    let mut used = Vec::new();
    Ok(LinearForm::constant(eval(expr, &vars, &mut used)?))
}

fn isolate_symbolic(form: &LinearForm, target: &str, coeff: f64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !near_zero(-form.constant) {
        parts.push(format_number(-form.constant));
    }
    for (name, c) in &form.coeffs {
        if name == target {
            continue;
        }
        let v = -*c;
        let term = if near_zero(v - 1.0) {
            name.clone()
        } else if near_zero(v + 1.0) {
            format!("-{}", name)
        } else {
            format!("{}{}", format_number(v), name)
        };
        parts.push(term);
    }
    if parts.is_empty() {
        parts.push("0".to_string());
    }
    let numerator = join_symbolic(parts);
    if near_zero(coeff - 1.0) {
        numerator
    } else {
        format!("({}) / {}", numerator, format_number(coeff))
    }
}

fn join_symbolic(parts: Vec<String>) -> String {
    let mut out = String::new();
    for part in parts {
        if out.is_empty() {
            out.push_str(&part);
        } else if let Some(rest) = part.strip_prefix('-') {
            out.push_str(" - ");
            out.push_str(rest);
        } else {
            out.push_str(" + ");
            out.push_str(&part);
        }
    }
    out
}

fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::Number(n) => format_number(*n),
        Expr::Var(name) => name.clone(),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus => format!("+{}", expr_to_string(expr)),
            UnaryOp::Minus => format!("-{}", expr_to_string(expr)),
        },
        Expr::Binary { op, left, right } => format!(
            "{} {} {}",
            expr_to_string(left),
            op_to_str(*op),
            expr_to_string(right)
        ),
        Expr::Func { name, arg } => format!("{}({})", name, expr_to_string(arg)),
        Expr::Group(inner) => expr_to_string(inner),
    }
}

fn op_to_str(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Pow => "^",
    }
}

fn near_zero(v: f64) -> bool {
    libm::fabs(v) < EPS
}

fn format_number(value: f64) -> String {
    let rounded = libm::round(value);
    if libm::fabs(value - rounded) < 1.0e-9 {
        return format!("{}", rounded as i64);
    }
    let mut s = format!("{:.10}", value);
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

pub mod history {
    extern crate alloc;

    use alloc::format;
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    // --- KV protocol constant validation (task 5) ---
    // Shell-side history uses these opcodes for the sunlight-kv service:
    //   KV_DELETE = 0x4B03
    //   KV_VALUE  = 0x4B05
    //   KV_PUT_SHM= 0x4B06
    //   KV_GET_SHM= 0x4B07
    //   KV_REPLY  = 0x4BFF
    //
    // Verified against sunlight-kv/src/main.rs (sunlightos cfg):
    //   const KV_DELETE: u64 = 0x4B03;
    //   const KV_GET: ...; const KV_PUT...; const KV_VALUE = 0x4B05;
    //   const KV_PUT_SHM = 0x4B06; const KV_GET_SHM = 0x4B07;
    //   const KV_REPLY = 0x4BFF; const KV_ERROR = 0x4BEE;
    // All match. Non-SHM KV ops exist on the service but history uses SHM for values.

    // Keep these keys short. KV_SHM packs keys into register IPC words starting
    // at word 2 (2 words × 8 bytes = 16 bytes max). raw_syscall_ipc carries only
    // words[0..4], so a key at word 2 has at most 16 transmitted bytes.
    // "calc.hist.idx" = 13 bytes and "calc.h." + 8 hex = 15 bytes, both fit.
    // Do NOT extend these keys beyond 16 bytes without adding SHM-key opcodes.
    const INDEX_KEY: &str = "calc.hist.idx";
    const RECORD_PREFIX: &str = "calc.h.";
    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    const MAX_RECORD: usize = 4096;

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    pub struct HistoryRecord {
        id: String,
        input: String,
        normalized: String,
        result: String,
        vars: Vec<String>,
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    impl HistoryRecord {
        pub fn new(input: &str, normalized: &str, result: &str, vars: &[String]) -> Self {
            Self {
                id: make_id(),
                input: input.to_string(),
                normalized: normalized.to_string(),
                result: result.to_string(),
                vars: vars.to_vec(),
            }
        }

        fn serialize(&self) -> String {
            let vars = self.vars.join(",");
            let mut s = format!(
                "id={}\ninput={}\nnormalized={}\nresult={}\nstatus=ok\ntimestamp={}\nvars={}\npid={}\n",
                self.id,
                escape(&self.input),
                escape(&self.normalized),
                escape(&self.result),
                timestamp(),
                escape(&vars),
                pid(),
            );
            if s.len() > MAX_RECORD {
                s.truncate(MAX_RECORD);
            }
            s
        }
    }

    pub fn handle_history_command(input: &str) -> Result<String, super::CalcError> {
        let args: Vec<&str> = input.split_ascii_whitespace().collect();
        match args.as_slice() {
            ["history"] => list(10),
            ["history", "clear"] => clear(),
            ["history", n] => {
                let limit = n
                    .parse::<usize>()
                    .map_err(|_| super::CalcError::new("invalid history limit"))?;
                list(limit)
            }
            ["history", "get", id] => get(id),
            _ => Err(super::CalcError::new("unknown history command")),
        }
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    pub fn calc_history_put(record: &HistoryRecord) -> Result<(), String> {
        let key = format!("{}{}", RECORD_PREFIX, record.id);
        kv_put(&key, record.serialize().as_bytes())?;
        let mut ids = read_index().unwrap_or_default();
        ids.push(record.id.clone());
        if ids.len() > 128 {
            ids.remove(0);
        }
        kv_put(INDEX_KEY, ids.join("\n").as_bytes())
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    pub fn calc_history_list(limit: usize) -> Result<Vec<String>, String> {
        let mut ids = read_index()?;
        ids.reverse();
        ids.truncate(limit);
        Ok(ids)
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    pub fn calc_history_get(id: &str) -> Result<String, String> {
        let key = format!("{}{}", RECORD_PREFIX, id);
        let bytes = kv_get(&key)?;
        core::str::from_utf8(&bytes)
            .map(|s| s.to_string())
            .map_err(|_| "history record is not utf8".to_string())
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    pub fn calc_history_clear() -> Result<(), String> {
        let ids = read_index().unwrap_or_default();
        for id in ids {
            let _ = kv_delete(&format!("{}{}", RECORD_PREFIX, id));
        }
        kv_put(INDEX_KEY, b"")
    }

    pub fn warn(msg: &str) {
        #[cfg(feature = "sunlight")]
        sunlight_ipc::debug_log(msg);
        #[cfg(not(feature = "sunlight"))]
        let _ = msg;
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn list(limit: usize) -> Result<String, super::CalcError> {
        match calc_history_list(limit) {
            Ok(ids) if ids.is_empty() => Ok("calc history: empty\n".to_string()),
            Ok(ids) => {
                let mut out = String::new();
                for id in ids {
                    match calc_history_get(&id) {
                        Ok(record) => {
                            let result = field(&record, "result").unwrap_or("");
                            let input = field(&record, "input").unwrap_or("");
                            out.push_str(&format!(
                                "{}  {} = {}\n",
                                id,
                                unescape(input),
                                unescape(result)
                            ));
                        }
                        Err(_) => out.push_str(&format!("{}  <unavailable>\n", id)),
                    }
                }
                Ok(out)
            }
            Err(_) => Ok("calc history unavailable\n".to_string()),
        }
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn get(id: &str) -> Result<String, super::CalcError> {
        match calc_history_get(id) {
            Ok(record) => Ok(record),
            Err(_) => Ok("calc history unavailable\n".to_string()),
        }
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn clear() -> Result<String, super::CalcError> {
        match calc_history_clear() {
            Ok(()) => Ok("calc history cleared\n".to_string()),
            Err(_) => Ok("calc history unavailable\n".to_string()),
        }
    }

    fn read_index() -> Result<Vec<String>, String> {
        match kv_get(INDEX_KEY) {
            Ok(bytes) => {
                let s =
                    core::str::from_utf8(&bytes).map_err(|_| "index is not utf8".to_string())?;
                Ok(s.lines()
                    .filter(|line| !line.is_empty())
                    .map(ToString::to_string)
                    .collect())
            }
            Err(e) if e == "not found" => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('\n', "\\n")
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut esc = false;
        for ch in s.chars() {
            if esc {
                out.push(if ch == 'n' { '\n' } else { ch });
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn field<'a>(record: &'a str, name: &str) -> Option<&'a str> {
        let prefix = format!("{}=", name);
        record
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
    }

    #[cfg(feature = "sunlight")]
    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn make_id() -> String {
        format!("{:08x}", sunlight_ipc::monotonic_millis() as u32)
    }

    #[cfg(not(feature = "sunlight"))]
    fn make_id() -> String {
        "host".to_string()
    }

    #[cfg(feature = "sunlight")]
    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn timestamp() -> u64 {
        sunlight_ipc::get_time_utc()
    }

    #[cfg(not(feature = "sunlight"))]
    fn timestamp() -> u64 {
        0
    }

    #[cfg(feature = "sunlight")]
    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn pid() -> u64 {
        sunlight_libc::getpid()
    }

    #[cfg(not(feature = "sunlight"))]
    fn pid() -> u64 {
        0
    }

    // History is a best-effort side effect. The calculator result must be returned
    // even when KV is missing, slow, or broken. Do not call blocking ipc_call()
    // here — this runs inside the interactive shell command path, which is called
    // synchronously by tty_server. A hang here freezes all keyboard input.
    //
    // Do not use sunlight-sm directly from calculator history. KV handles its own
    // persistence as a background operation; callers only talk to KV.
    #[cfg(feature = "sunlight")]
    const KV_LOOKUP_TIMEOUT_MS: u64 = 250;
    #[cfg(feature = "sunlight")]
    const KV_OP_TIMEOUT_MS: u64 = 250;
    #[cfg(feature = "sunlight")]
    static mut KV_CAP_CACHE: sunlight_ipc::CapabilityToken = sunlight_ipc::CapabilityToken::INVALID;

    #[cfg(feature = "sunlight")]
    fn kv_cap() -> Result<sunlight_ipc::CapabilityToken, String> {
        use sunlight_ipc::{nameserver_lookup_timeout, CapabilityToken};

        let cached = unsafe { KV_CAP_CACHE };
        if cached != CapabilityToken::INVALID {
            return Ok(cached);
        }

        match nameserver_lookup_timeout("sunlight-kv", KV_LOOKUP_TIMEOUT_MS) {
            Some(cap) => {
                unsafe {
                    KV_CAP_CACHE = cap;
                }
                Ok(cap)
            }
            None => {
                sunlight_ipc::debug_log("[CALC-KV] lookup sunlight-kv failed/timeout");
                Err("sunlight-kv unavailable".to_string())
            }
        }
    }

    #[cfg(feature = "sunlight")]
    fn kv_call_checked(
        cap: sunlight_ipc::CapabilityToken,
        msg: sunlight_ipc::IpcMsg,
    ) -> Result<sunlight_ipc::IpcMsg, String> {
        sunlight_ipc::ipc_call_timeout(cap, msg, KV_OP_TIMEOUT_MS)
            .map_err(|e| format!("kv ipc failed: {:?}", e))
    }

    #[cfg(feature = "sunlight")]
    fn kv_put(key: &str, value: &[u8]) -> Result<(), String> {
        use sunlight_ipc::{shm_alloc, shm_free, IpcMsg};
        const KV_PUT_SHM: u64 = 0x4B06;
        const KV_REPLY: u64 = 0x4BFF;
        const SHM_PAGE: usize = 4096;

        // KV protocol constants (shell side) verified against sunlight-kv/src/main.rs:
        // KV_PUT_SHM=0x4B06, KV_REPLY=0x4BFF

        // TODO: add KV_PUT_SHM2 with key+value in shared memory so keys are not
        // constrained by the 4-word register IPC ABI.
        ensure_register_key_len(2, key)?;
        if value.len() > SHM_PAGE {
            return Err("value too large".to_string());
        }

        let cap = kv_cap()?;

        let (ptr, tok) = match shm_alloc() {
            Ok(v) => v,
            Err(_) => return Err("shm alloc failed".to_string()),
        };
        unsafe {
            core::ptr::copy_nonoverlapping(value.as_ptr(), ptr, value.len());
        }

        let mut msg = IpcMsg::with_label(KV_PUT_SHM)
            .word(0, value.len() as u64)
            .with_cap(0, tok);
        pack_str_register_words(&mut msg, 2, key)?;

        // IMPORTANT: for PUT, the caller allocates the shared page and must free
        // the token on *all* exit paths after the IPC attempt (success, error,
        // timeout). The kernel returns WouldBlock to userspace, so a timeout
        // here means we may or may not have delivered the grant; we still own
        // and must release the local mapping.
        let reply_res = kv_call_checked(cap, msg);
        let _ = shm_free(tok);

        let reply = match reply_res {
            Ok(r) => r,
            Err(e) => {
                sunlight_ipc::debug_log(&format!("[CALC-KV] put timeout/error key={}", key));
                return Err(e);
            }
        };

        if reply.label == KV_REPLY && reply.words[0] == 0 {
            Ok(())
        } else {
            Err("put failed".to_string())
        }
    }

    #[cfg(not(feature = "sunlight"))]
    fn kv_put(_key: &str, _value: &[u8]) -> Result<(), String> {
        Err("sunlight-kv unavailable".to_string())
    }

    #[cfg(feature = "sunlight")]
    fn kv_get(key: &str) -> Result<Vec<u8>, String> {
        use sunlight_ipc::{shm_free, shm_map, CapabilityToken, IpcMsg};
        const KV_GET_SHM: u64 = 0x4B07;
        const KV_VALUE: u64 = 0x4B05;
        const SHM_PAGE: usize = 4096;

        // KV protocol constants (shell side) verified against sunlight-kv/src/main.rs:
        // KV_GET_SHM=0x4B07, KV_VALUE=0x4B05

        // TODO: add KV_GET_SHM2 with the key in shared memory to remove the
        // current 16-byte register-IPC key limit.
        ensure_register_key_len(2, key)?;

        let cap = kv_cap()?;

        let mut msg = IpcMsg::with_label(KV_GET_SHM);
        pack_str_register_words(&mut msg, 2, key)?;

        let reply = match kv_call_checked(cap, msg) {
            Ok(r) => r,
            Err(e) => {
                sunlight_ipc::debug_log(&format!("[CALC-KV] get timeout/error key={}", key));
                return Err(e);
            }
        };

        if reply.label != KV_VALUE {
            // Service replied with non-VALUE (e.g. not found or KV_ERROR).
            // Per protocol: missing keys must produce a reply (not silence).
            if reply.label == 0x4BEE
            /* KV_ERROR */
            {
                return Err("not found".to_string());
            }
            return Err("not found".to_string());
        }

        let n = (reply.words[0] as usize).min(SHM_PAGE);
        let tok = reply.caps[0];
        if tok == CapabilityToken::INVALID {
            return Ok(Vec::new());
        }

        // For GET: the service returns a fresh SHM token in caps[0].
        // Caller must map, copy, then free. Free on map failure too.
        // Ownership comment: caller frees the token returned by the service.
        let ptr = match shm_map(tok) {
            Ok(p) => p,
            Err(_) => {
                let _ = shm_free(tok);
                return Err("shm map failed".to_string());
            }
        };
        let value = unsafe { core::slice::from_raw_parts(ptr, n) }.to_vec();
        let _ = shm_free(tok);
        Ok(value)
    }

    #[cfg(not(feature = "sunlight"))]
    fn kv_get(_key: &str) -> Result<Vec<u8>, String> {
        Err("sunlight-kv unavailable".to_string())
    }

    #[cfg(feature = "sunlight")]
    #[cfg_attr(feature = "sunlight", allow(dead_code))]
    fn kv_delete(key: &str) -> Result<(), String> {
        use sunlight_ipc::IpcMsg;
        const KV_DELETE: u64 = 0x4B03;
        const KV_REPLY: u64 = 0x4BFF;
        // KV protocol constants (shell side) verified against sunlight-kv/src/main.rs:
        // KV_DELETE=0x4B03, KV_REPLY=0x4BFF

        ensure_register_key_len(1, key)?;
        let cap = kv_cap()?;
        let mut msg = IpcMsg::with_label(KV_DELETE);
        msg.words[0] = key.len() as u64;
        pack_str_register_words(&mut msg, 1, key)?;
        let reply = match kv_call_checked(cap, msg) {
            Ok(r) => r,
            Err(e) => {
                sunlight_ipc::debug_log(&format!("[CALC-KV] delete timeout/error key={}", key));
                return Err(e);
            }
        };
        if reply.label == KV_REPLY && reply.words[0] == 0 {
            Ok(())
        } else {
            Err("delete failed".to_string())
        }
    }

    #[cfg(not(feature = "sunlight"))]
    fn kv_delete(_key: &str) -> Result<(), String> {
        Err("sunlight-kv unavailable".to_string())
    }

    #[cfg(feature = "sunlight")]
    fn pack_str_register_words(
        msg: &mut sunlight_ipc::IpcMsg,
        start_word: usize,
        s: &str,
    ) -> Result<(), String> {
        ensure_register_key_len(start_word, s)?;
        let bytes = s.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            let wi = start_word + i / 8;
            let shift = (i % 8) * 8;
            msg.words[wi] |= (b as u64) << shift;
        }
        Ok(())
    }

    #[cfg(feature = "sunlight")]
    fn ensure_register_key_len(start_word: usize, s: &str) -> Result<(), String> {
        if start_word >= sunlight_ipc::IPC_REGISTER_WORDS {
            return Err("invalid register ipc word offset".to_string());
        }
        let max = (sunlight_ipc::IPC_REGISTER_WORDS - start_word) * 8;
        if s.len() > max {
            return Err(format!(
                "key too long for register ipc: len={} max={}",
                s.len(),
                max
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_line(input: &str) -> String {
        let mut session = CalcSession::new();
        session.run_command(input)
    }

    #[test]
    fn required_eval_cases() {
        let cases = [
            ("8 * 2", "16\n"),
            ("2 + 2 * 3", "8\n"),
            ("(2 + 2) * 3", "12\n"),
            ("2^3^2", "512\n"),
            ("10 % 3", "1\n"),
            ("-2^2", "-4\n"),
            ("(-2)^2", "4\n"),
            ("2(3+4)", "14\n"),
            ("(8+2)(6+5)(2+2)", "440\n"),
            ("(2*1)/(8+2)(6+5)(2+2)", "8.8\n"),
            ("sqrt(abs(-9)) + cos(0)", "4\n"),
        ];
        for (input, expected) in cases {
            assert_eq!(eval_line(input), expected, "{input}");
        }
    }

    #[test]
    fn variables_are_session_local() {
        let mut session = CalcSession::new();
        assert_eq!(session.run_command("x = 2 + 2 - 4"), "x = 0\n");
        assert_eq!(session.run_command("y = 10"), "y = 10\n");
        assert_eq!(session.run_command("12x + 10y - 10"), "90\n");
        assert_eq!(session.run_command("x(y + 1)"), "0\n");
    }

    #[test]
    fn solve_linear_equation() {
        assert_eq!(eval_line("solve 2x + 4 = 10"), "x = 3\n");
    }

    #[test]
    fn errors_are_friendly() {
        assert_eq!(
            eval_line("2 +"),
            "calc error: expected expression after '+'\n"
        );
        assert_eq!(eval_line("sqrt(-1)"), "calc error: sqrt domain error\n");
        assert_eq!(eval_line("1 / 0"), "calc error: division by zero\n");
        assert_eq!(
            eval_line("unknown_var + 2"),
            "calc error: unknown variable 'unknown_var'\n"
        );
    }

    // --- History / KV graceful behavior (tasks 4, 7) ---
    #[test]
    fn calc_result_is_printed_regardless_of_history() {
        // "= 8 * 2" must print 16 even if history save fails (no KV in this test cfg).
        assert_eq!(eval_line("8 * 2"), "16\n");
    }

    #[test]
    fn history_reports_unavailable_or_empty_gracefully() {
        // Without a running sunlight-kv (non-sunlight test build), kv ops fail.
        // Must not panic or alter calc results.
        let h = eval_line("history");
        // Under real sunlight+timeout, unavailable or timeout from KV also
        // yields the short "calc history unavailable\n" (see list()).
        assert!(
            h == "calc history: empty\n" || h.contains("unavailable"),
            "unexpected history output: {:?}",
            h
        );
    }

    #[test]
    fn calc_does_not_hang_on_history_failure() {
        // Multiple calcs must complete promptly even when every history op errors.
        assert_eq!(eval_line("2 + 2"), "4\n");
        assert_eq!(eval_line("3 * 3"), "9\n");
        let _ = eval_line("history");
    }
}
