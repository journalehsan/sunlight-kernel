use crate::Value;
#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: E,
    pub pos: usize,
}
#[derive(Clone, Debug)]
pub enum E {
    Literal(Value),
    Id(String),
    This,
    Group(Box<Expr>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Member(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Function(Option<String>, Vec<String>, Vec<Stmt>, FunctionKind),
    Object(Vec<(String, Expr)>),
    Array(Vec<Expr>),
}
#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: S,
    pub pos: usize,
}
#[derive(Clone, Debug)]
pub enum S {
    Empty,
    Expr(Expr),
    Block(Vec<Stmt>),
    Decl(bool, Vec<(String, Option<Expr>)>),
    Function(String, Expr),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    Return(Option<Expr>),
    Break,
    Continue,
    Throw(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKind {
    Ordinary,
    Arrow,
    Method,
}
