//! Shared scripting interpreter. Platform runtimes own descriptors and processes.
use crate::{
    expansion::{expand_word, valid_name},
    parser::{AstNode, Quote, RedirectMode, Redirection, SimpleCommand, Word},
    script_parser::{self, Command, CommandKind},
    shellenv::ShellEnv,
};
use alloc::{
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};

pub const MAX_CALL_DEPTH: usize = 16;
pub const MAX_FUNCTIONS: usize = 64;
pub const MAX_ARGUMENTS: usize = 256;
#[derive(Clone)]
pub struct ShellExecutionContext {
    pub cwd: String,
    pub env: ShellEnv,
    pub last_status: i32,
    pub functions: BTreeMap<String, Rc<Command>>,
    /// Borrowed descriptors; redirection scopes close only descriptors they open.
    pub io: [i32; 3],
    pub loop_depth: usize,
    pub call_depth: usize,
    pub source_depth: usize,
    pub interactive: bool,
}
impl ShellExecutionContext {
    pub fn new(env: ShellEnv, cwd: String) -> Self {
        Self {
            cwd,
            env,
            last_status: 0,
            functions: BTreeMap::new(),
            io: [0, 1, 2],
            loop_depth: 0,
            call_depth: 0,
            source_depth: 0,
            interactive: false,
        }
    }
    fn status(&mut self, status: i32) {
        self.last_status = status;
        self.env.set("?", &status.to_string());
    }
}
#[derive(Debug)]
pub struct Error {
    pub status: i32,
    pub message: String,
}
impl Error {
    pub fn new(status: i32, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Status(i32),
    Break(usize),
    Continue(usize),
    Return(i32),
    Exit(i32),
}
/// No output capture is performed by the interpreter. Writes and pipelines stream.
pub trait Runtime {
    fn external(
        &mut self,
        command: &AstNode<String>,
        context: &ShellExecutionContext,
    ) -> Result<i32, Error>;
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<(), Error>;
    fn open(
        &mut self,
        path: &str,
        mode: RedirectMode,
        context: &ShellExecutionContext,
    ) -> Result<i32, Error>;
    fn close(&mut self, fd: i32);
    fn directory(&mut self, path: &str, context: &ShellExecutionContext) -> Result<String, Error>;
    fn source(&mut self, path: &str, context: &ShellExecutionContext) -> Result<String, Error>;
}
pub fn execute(
    source: &str,
    context: &mut ShellExecutionContext,
    runtime: &mut impl Runtime,
) -> Result<Flow, Error> {
    let tree = script_parser::parse(source).map_err(|e| {
        let (line, column) = e.location(source);
        Error::new(2, format!("{line}:{column}: {}", e.message))
    })?;
    run(&tree, context, runtime)
}
fn expand(word: &Word, context: &ShellExecutionContext, split: bool) -> Result<Vec<String>, Error> {
    expand_word(word, &context.env, split).map_err(|e| Error::new(2, e.0))
}
fn scalar(word: &Word, context: &ShellExecutionContext) -> Result<String, Error> {
    Ok(expand(word, context, false)?
        .into_iter()
        .next()
        .unwrap_or_default())
}
fn redirects(
    redirs: &[Redirection],
    context: &mut ShellExecutionContext,
    runtime: &mut impl Runtime,
) -> Result<Vec<i32>, Error> {
    let mut opened = Vec::new();
    for redir in redirs {
        let result = match redir {
            Redirection::Dup { fd, source } => {
                context.io[*fd as usize] = context.io[*source as usize];
                Ok(())
            }
            Redirection::Open { fd, mode, path } => scalar(path, context)
                .and_then(|path| runtime.open(&path, *mode, context))
                .map(|handle| {
                    opened.push(handle);
                    context.io[*fd as usize] = handle;
                }),
        };
        if let Err(e) = result {
            for fd in opened {
                runtime.close(fd);
            }
            return Err(e);
        }
    }
    Ok(opened)
}
fn run(
    node: &Command,
    context: &mut ShellExecutionContext,
    runtime: &mut impl Runtime,
) -> Result<Flow, Error> {
    // Function-definition redirects are evaluated at invocation, never definition.
    let saved = context.io;
    let opened = if matches!(node.kind, CommandKind::FunctionDefinition { .. }) {
        Vec::new()
    } else {
        match redirects(&node.redirections, context, runtime) {
            Ok(fds) => fds,
            Err(e) => {
                context.io = saved;
                return Err(e);
            }
        }
    };
    let result = run_kind(node, context, runtime);
    context.io = saved;
    for fd in opened {
        runtime.close(fd);
    }
    match &result {
        Ok(Flow::Status(s) | Flow::Return(s) | Flow::Exit(s)) => context.status(*s),
        Err(e) => context.status(e.status),
        _ => {}
    }
    result
}
fn run_kind(
    node: &Command,
    context: &mut ShellExecutionContext,
    runtime: &mut impl Runtime,
) -> Result<Flow, Error> {
    use CommandKind::*;
    match &node.kind {
        SimpleCommand(command) => simple(command, context, runtime),
        SequentialList(commands) => {
            let mut status = 0;
            for command in commands {
                match run(command, context, runtime)? {
                    Flow::Status(s) => status = s,
                    flow => return Ok(flow),
                }
            }
            Ok(Flow::Status(status))
        }
        AndOrList { first, rest } => {
            let mut status = match run(first, context, runtime)? {
                Flow::Status(s) => s,
                flow => return Ok(flow),
            };
            for (and, command) in rest {
                if (*and && status == 0) || (!*and && status != 0) {
                    match run(command, context, runtime)? {
                        Flow::Status(s) => status = s,
                        flow => return Ok(flow),
                    }
                }
            }
            Ok(Flow::Status(status))
        }
        NegatedPipeline(command) => match run(command, context, runtime)? {
            Flow::Status(s) => Ok(Flow::Status(i32::from(s == 0))),
            flow => Ok(flow),
        },
        Pipeline(commands) => {
            let mut expanded = Vec::new();
            for command in commands {
                let SimpleCommand(simple) = &command.kind else {
                    return Err(Error::new(
                        2,
                        "compound pipeline stages are not implemented",
                    ));
                };
                if simple.argv.first().and_then(literal).is_some_and(|name| {
                    context.functions.contains_key(name)
                        || is_special(name)
                        || matches!(name, "break" | "continue")
                }) {
                    return Err(Error::new(
                        2,
                        "function and scripting builtin pipeline stages are not implemented",
                    ));
                }
                if simple.argv.first().is_some_and(|w| assignment(w).is_some()) {
                    return Err(Error::new(
                        2,
                        "pipeline prefix assignments are not implemented",
                    ));
                }
                let stage = expanded_simple(simple, context)?;
                if stage.argv.first().is_none_or(|name| {
                    context.functions.contains_key(name)
                        || is_special(name)
                        || matches!(name.as_str(), "break" | "continue" | "test" | "[" | "read")
                }) {
                    return Err(Error::new(
                        2,
                        "empty, function or scripting builtin pipeline stage is not implemented",
                    ));
                }
                expanded.push(stage);
            }
            Ok(Flow::Status(
                runtime.external(&AstNode::Pipeline(expanded), context)?,
            ))
        }
        IfCommand {
            branches,
            otherwise,
        } => {
            for (condition, body) in branches {
                match run(condition, context, runtime)? {
                    Flow::Status(0) => return run(body, context, runtime),
                    Flow::Status(_) => {}
                    flow => return Ok(flow),
                }
            }
            match otherwise {
                Some(body) => run(body, context, runtime),
                None => Ok(Flow::Status(0)),
            }
        }
        ForCommand { name, words, body } => {
            let mut values = Vec::new();
            if let Some(words) = words {
                for word in words {
                    values.extend(expand(word, context, true)?);
                    if values.len() > MAX_ARGUMENTS {
                        return Err(Error::new(2, "for word limit exceeded"));
                    }
                }
            } else {
                values = context.env.positional.clone();
            }
            context.loop_depth += 1;
            let result = (|| {
                let mut status = 0;
                for value in values {
                    context
                        .env
                        .assign(name, &value)
                        .map_err(|e| Error::new(2, e))?;
                    match run(body, context, runtime)? {
                        Flow::Status(s) => status = s,
                        Flow::Break(1) => break,
                        Flow::Continue(1) => continue,
                        Flow::Break(n) => return Ok(Flow::Break(n - 1)),
                        Flow::Continue(n) => return Ok(Flow::Continue(n - 1)),
                        flow => return Ok(flow),
                    }
                }
                Ok(Flow::Status(status))
            })();
            context.loop_depth -= 1;
            result
        }
        WhileCommand { condition, body } | UntilCommand { condition, body } => {
            let until = matches!(node.kind, UntilCommand { .. });
            context.loop_depth += 1;
            let result = (|| {
                let mut status = 0;
                loop {
                    match run(condition, context, runtime)? {
                        Flow::Status(s) if (s == 0) == until => break,
                        Flow::Status(_) => {}
                        Flow::Break(1) => break,
                        Flow::Continue(1) => continue,
                        Flow::Break(n) => return Ok(Flow::Break(n - 1)),
                        Flow::Continue(n) => return Ok(Flow::Continue(n - 1)),
                        flow => return Ok(flow),
                    }
                    match run(body, context, runtime)? {
                        Flow::Status(s) => status = s,
                        Flow::Break(1) => break,
                        Flow::Continue(1) => continue,
                        Flow::Break(n) => return Ok(Flow::Break(n - 1)),
                        Flow::Continue(n) => return Ok(Flow::Continue(n - 1)),
                        flow => return Ok(flow),
                    }
                }
                Ok(Flow::Status(status))
            })();
            context.loop_depth -= 1;
            result
        }
        CaseCommand { word, arms } => {
            let value = scalar(word, context)?;
            for (patterns, body) in arms {
                for pattern in patterns {
                    let pattern = crate::expansion::case_pattern(pattern, &context.env)
                        .map_err(|e| Error::new(2, e.0))?;
                    if pattern_matches(&pattern, &value)? {
                        return run(body, context, runtime);
                    }
                }
            }
            Ok(Flow::Status(0))
        }
        FunctionDefinition { name, body } => {
            if is_special(name) {
                return Err(Error::new(2, "special builtin cannot be a function name"));
            }
            if context.functions.len() >= MAX_FUNCTIONS && !context.functions.contains_key(name) {
                return Err(Error::new(2, "function limit exceeded"));
            }
            let mut body = (**body).clone();
            body.redirections.extend(node.redirections.clone());
            context.functions.insert(name.clone(), Rc::new(body));
            Ok(Flow::Status(0))
        }
        BraceGroup(body) => run(body, context, runtime),
        Subshell(body) => {
            let mut child = context.clone();
            child.loop_depth = 0;
            match run(body, &mut child, runtime)? {
                Flow::Exit(s) | Flow::Return(s) | Flow::Status(s) => Ok(Flow::Status(s)),
                _ => Err(Error::new(2, "invalid subshell control transfer")),
            }
        }
    }
}
fn literal(word: &Word) -> Option<&str> {
    if word.parts.len() == 1 && word.parts[0].quote == Quote::Unquoted {
        Some(&word.parts[0].text)
    } else {
        None
    }
}
fn assignment(word: &Word) -> Option<(String, Word)> {
    let part = word.parts.first()?;
    if part.quote != Quote::Unquoted {
        return None;
    }
    let (name, rest) = part.text.split_once('=')?;
    if !valid_name(name) {
        return None;
    }
    let mut value = word.clone();
    value.parts[0].text = String::from(rest);
    Some((String::from(name), value))
}
fn expanded_simple(
    command: &SimpleCommand,
    context: &ShellExecutionContext,
) -> Result<SimpleCommand<String>, Error> {
    let mut argv = Vec::new();
    for word in &command.argv {
        let declaration = argv
            .first()
            .is_some_and(|name: &String| matches!(name.as_str(), "export" | "readonly"))
            && assignment(word).is_some();
        argv.extend(expand(word, context, !declaration)?);
        if argv.len() > MAX_ARGUMENTS {
            return Err(Error::new(2, "argument limit exceeded"));
        }
    }
    let mut redirections = Vec::new();
    for redir in &command.redirections {
        redirections.push(match redir {
            Redirection::Dup { fd, source } => Redirection::Dup {
                fd: *fd,
                source: *source,
            },
            Redirection::Open { fd, mode, path } => Redirection::Open {
                fd: *fd,
                mode: *mode,
                path: scalar(path, context)?,
            },
        });
    }
    Ok(SimpleCommand { argv, redirections })
}
fn simple(
    command: &SimpleCommand,
    context: &mut ShellExecutionContext,
    runtime: &mut impl Runtime,
) -> Result<Flow, Error> {
    let prefix = command
        .argv
        .iter()
        .take_while(|w| assignment(w).is_some())
        .count();
    // Command arguments expand against the original environment, before prefix assignments.
    let mut expanded = expanded_simple(
        &SimpleCommand {
            argv: command.argv[prefix..].to_vec(),
            redirections: Vec::new(),
        },
        context,
    )?;
    let name = expanded.argv.first().map(String::as_str).unwrap_or("");
    let permanent = expanded.argv.is_empty() || is_special(name);
    let saved_env = (!permanent && prefix != 0).then(|| context.env.clone());
    let result = (|| {
        for word in &command.argv[..prefix] {
            let (key, value) = assignment(word).unwrap();
            let value = scalar(&value, context)?;
            context
                .env
                .assign(&key, &value)
                .map_err(|e| Error::new(2, e))?;
            if !permanent {
                context.env.export(&key);
            }
        }
        let saved_io = context.io;
        let opened = match redirects(&command.redirections, context, runtime) {
            Ok(fds) => fds,
            Err(e) => {
                context.io = saved_io;
                return Err(e);
            }
        };
        let result = if expanded.argv.is_empty() {
            Ok(Flow::Status(0))
        } else {
            dispatch(&mut expanded, context, runtime)
        };
        let result = match result {
            Err(e)
                if expanded
                    .argv
                    .first()
                    .is_some_and(|name| matches!(name.as_str(), "cd" | "test" | "[")) =>
            {
                runtime
                    .write(context.io[2], format!("sshl: {}\n", e.message).as_bytes())
                    .map(|_| Flow::Status(e.status))
            }
            other => other,
        };
        context.io = saved_io;
        for fd in opened {
            runtime.close(fd);
        }
        result
    })();
    if let Some(env) = saved_env {
        for word in &command.argv[..prefix] {
            let (key, _) = assignment(word).unwrap();
            context.env.restore_variable(&key, &env);
        }
    }
    result
}
pub fn is_special(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "continue"
            | ":"
            | "."
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "times"
            | "trap"
            | "unset"
    )
}
fn dispatch(
    command: &mut SimpleCommand<String>,
    context: &mut ShellExecutionContext,
    runtime: &mut impl Runtime,
) -> Result<Flow, Error> {
    let name = command.argv[0].as_str();
    let args = &command.argv[1..];
    if let Some(body) = context.functions.get(name).cloned() {
        if context.call_depth >= MAX_CALL_DEPTH {
            return Err(Error::new(2, "function recursion exceeds 16 calls"));
        }
        let saved = core::mem::replace(&mut context.env.positional, args.to_vec());
        context.call_depth += 1;
        let result = run(&body, context, runtime);
        context.call_depth -= 1;
        context.env.positional = saved;
        return result.map(|flow| match flow {
            Flow::Return(s) => Flow::Status(s),
            other => other,
        });
    }
    let success = Flow::Status(0);
    match name {
        ":" | "true" => Ok(success),
        "false" => Ok(Flow::Status(1)),
        "exit" | "return" => {
            if args.len() > 1 {
                return Err(Error::new(2, "too many status arguments"));
            }
            let status = if let Some(arg) = args.first() {
                arg.parse::<i32>()
                    .map_err(|_| Error::new(2, "invalid exit status"))?
                    .rem_euclid(256)
            } else {
                context.last_status
            };
            if name == "return" {
                if context.call_depth == 0 && context.source_depth == 0 {
                    Err(Error::new(2, "return outside function or sourced file"))
                } else {
                    Ok(Flow::Return(status))
                }
            } else {
                Ok(Flow::Exit(status))
            }
        }
        "break" | "continue" => {
            if args.len() > 1 {
                return Err(Error::new(2, "too many loop-control arguments"));
            }
            let levels = args
                .first()
                .map(|s| s.parse::<usize>())
                .transpose()
                .map_err(|_| Error::new(2, "invalid loop count"))?
                .unwrap_or(1);
            if levels == 0 || context.loop_depth == 0 {
                return Err(Error::new(2, "loop control outside loop or invalid count"));
            }
            let levels = levels.min(context.loop_depth);
            Ok(if name == "break" {
                Flow::Break(levels)
            } else {
                Flow::Continue(levels)
            })
        }
        "export" | "readonly" | "unset" => {
            if args.is_empty() {
                return Err(Error::new(
                    2,
                    "attribute listing is not implemented; supply variable names",
                ));
            }
            for arg in args {
                let (key, value) = arg
                    .split_once('=')
                    .map(|(k, v)| (k, Some(v)))
                    .unwrap_or((arg, None));
                if !valid_name(key) || (name == "unset" && value.is_some()) {
                    return Err(Error::new(2, "invalid variable name"));
                }
                if name == "unset" {
                    if context.env.is_readonly(key) {
                        return Err(Error::new(2, "variable is readonly"));
                    }
                    context.env.unset(key);
                } else {
                    if let Some(value) = value {
                        context
                            .env
                            .assign(key, value)
                            .map_err(|e| Error::new(2, e))?;
                    } else if context.env.get(key).is_none() {
                        context.env.assign(key, "").map_err(|e| Error::new(2, e))?;
                    }
                    if name == "export" {
                        context.env.export(key);
                    } else {
                        context.env.make_readonly(key);
                    }
                }
            }
            Ok(success)
        }
        "set" => {
            if args.first().is_some_and(|a| a == "--") {
                context.env.positional = args[1..].to_vec();
                Ok(success)
            } else {
                Err(Error::new(
                    2,
                    "only set -- positional parameters is implemented",
                ))
            }
        }
        "shift" => {
            if args.len() > 1 {
                return Err(Error::new(2, "too many shift arguments"));
            }
            let n = args
                .first()
                .map(|s| s.parse::<usize>())
                .transpose()
                .map_err(|_| Error::new(2, "invalid shift count"))?
                .unwrap_or(1);
            if n > context.env.positional.len() {
                return Ok(Flow::Status(1));
            }
            context.env.positional.drain(..n);
            Ok(success)
        }
        "echo" => {
            runtime.write(context.io[1], args.join(" ").as_bytes())?;
            runtime.write(context.io[1], b"\n")?;
            Ok(success)
        }
        "pwd" => {
            runtime.write(context.io[1], context.cwd.as_bytes())?;
            runtime.write(context.io[1], b"\n")?;
            Ok(success)
        }
        "cd" => {
            if args.len() > 1 {
                return Err(Error::new(2, "too many cd arguments"));
            }
            let target = args
                .first()
                .map(String::as_str)
                .unwrap_or_else(|| context.env.get("HOME").unwrap_or("/"));
            let next = runtime.directory(target, context)?;
            context
                .env
                .assign("OLDPWD", &context.cwd)
                .map_err(|e| Error::new(2, e))?;
            context
                .env
                .assign("PWD", &next)
                .map_err(|e| Error::new(2, e))?;
            context.cwd = next;
            Ok(success)
        }
        "test" | "[" => {
            let args = if name == "[" {
                if args.last().is_none_or(|s| s != "]") {
                    return Err(Error::new(2, "missing ]"));
                }
                &args[..args.len() - 1]
            } else {
                args
            };
            test(args).map(|value| Flow::Status(i32::from(!value)))
        }
        "eval" | "." => {
            if context.source_depth >= MAX_CALL_DEPTH {
                return Err(Error::new(2, "source/eval nesting exceeds 16 levels"));
            }
            let source = if name == "eval" {
                args.join(" ")
            } else {
                let path = args
                    .first()
                    .ok_or_else(|| Error::new(2, "missing source filename"))?;
                runtime.source(path, context)?
            };
            context.source_depth += 1;
            let result = execute(&source, context, runtime);
            context.source_depth -= 1;
            if name == "." {
                result.map(|flow| match flow {
                    Flow::Return(s) => Flow::Status(s),
                    other => other,
                })
            } else {
                result
            }
        }
        "exec" | "times" | "trap" | "read" => {
            Err(Error::new(2, "this scripting builtin is not implemented"))
        }
        _ => runtime
            .external(&AstNode::Command(command.clone()), context)
            .map(Flow::Status),
    }
}
fn test(args: &[String]) -> Result<bool, Error> {
    match args {
        [] => Ok(false),
        [value] => Ok(!value.is_empty()),
        [op, value] if op == "!" => Ok(value.is_empty()),
        [op, value] if op == "-n" => Ok(!value.is_empty()),
        [op, value] if op == "-z" => Ok(value.is_empty()),
        [a, op, b] => match op.as_str() {
            "=" => Ok(a == b),
            "!=" => Ok(a != b),
            "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                let a = a
                    .parse::<i64>()
                    .map_err(|_| Error::new(2, "test: invalid integer"))?;
                let b = b
                    .parse::<i64>()
                    .map_err(|_| Error::new(2, "test: invalid integer"))?;
                Ok(match op.as_str() {
                    "-eq" => a == b,
                    "-ne" => a != b,
                    "-lt" => a < b,
                    "-le" => a <= b,
                    "-gt" => a > b,
                    _ => a >= b,
                })
            }
            _ => Err(Error::new(2, "test operator is not implemented")),
        },
        _ => Err(Error::new(2, "test expression is not implemented")),
    }
}
fn pattern_matches(pattern: &[(char, bool)], value: &str) -> Result<bool, Error> {
    if pattern.iter().any(|(c, quoted)| *c == '[' && !quoted) {
        return Err(Error::new(2, "case bracket patterns are not implemented"));
    }
    let value: Vec<char> = value.chars().collect();
    let (mut p, mut v, mut star) = (0, 0, None);
    while v < value.len() {
        if p < pattern.len() && pattern[p] == ('*', false) {
            star = Some((p, v));
            p += 1;
        } else if p < pattern.len()
            && (pattern[p].0 == value[v] || (pattern[p].0 == '?' && !pattern[p].1))
        {
            p += 1;
            v += 1;
        } else if let Some((s, matched)) = star {
            v = matched + 1;
            p = s + 1;
            star = Some((s, v));
        } else {
            return Ok(false);
        }
    }
    while p < pattern.len() && pattern[p] == ('*', false) {
        p += 1;
    }
    Ok(p == pattern.len())
}
