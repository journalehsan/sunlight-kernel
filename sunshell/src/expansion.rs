//! Ordered expansion stages shared by native and host execution.
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use crate::parser::{AstNode, Quote, Redirection, SimpleCommand, Word};
use crate::shellenv::ShellEnv;

#[derive(Debug)]
pub struct ExpansionError(pub &'static str);

// None is a quoted empty anchor; it survives splitting and is removed last.
#[derive(Clone, Copy)]
struct Atom { ch: Option<char>, split: bool, quoted: bool }
type Field = Vec<Atom>;

pub fn expand_ast(ast: &AstNode, env: &ShellEnv) -> Result<AstNode<String>, ExpansionError> {
    fn command(cmd: &SimpleCommand, env: &ShellEnv) -> Result<SimpleCommand<String>, ExpansionError> {
        let mut argv = Vec::new();
        for word in &cmd.argv { argv.extend(expand_word(word, env, true)?); }
        let mut redirections = Vec::new();
        for redir in &cmd.redirections {
            redirections.push(match redir {
                Redirection::Dup { fd, source } => Redirection::Dup { fd: *fd, source: *source },
                Redirection::Open { fd, mode, path } => {
                    // POSIX redirection words are not subject to field splitting.
                    let mut fields = expand_word(path, env, false)?;
                    Redirection::Open { fd: *fd, mode: *mode, path: fields.remove(0) }
                }
            });
        }
        Ok(SimpleCommand { argv, redirections })
    }
    Ok(match ast {
        AstNode::Command(cmd) => AstNode::Command(command(cmd, env)?),
        AstNode::Pipeline(cmds) => AstNode::Pipeline(cmds.iter().map(|cmd| command(cmd, env)).collect::<Result<_, _>>()?),
    })
}

pub fn expand_word(word: &Word, env: &ShellEnv, split: bool) -> Result<Vec<String>, ExpansionError> {
    let atoms = parameter_expansion(word, env)?;
    let fields = if split { field_splitting(atoms, env.get("IFS").unwrap_or(" \t\n")) } else { alloc::vec![atoms] };
    let fields = pathname_expansion(fields)?;
    Ok(quote_removal(fields))
}

fn parameter_expansion(word: &Word, env: &ShellEnv) -> Result<Field, ExpansionError> {
    let mut atoms = Vec::new();
    for part in &word.parts {
        let quoted = part.quote != Quote::Unquoted;
        if quoted { atoms.push(Atom { ch: None, split: false, quoted: true }); }
        let expandable = matches!(part.quote, Quote::Unquoted | Quote::Double);
        let mut chars = part.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if expandable && ch == '`' { return Err(ExpansionError("command substitution is unsupported")); }
            if expandable && ch == '$' {
                let mut key = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    while chars.peek().is_some_and(|c| *c != '}') { key.push(chars.next().unwrap()); }
                    if chars.next() != Some('}') { return Err(ExpansionError("unterminated parameter expansion")); }
                    if key != "?" && !valid_name(&key) { return Err(ExpansionError("parameter operators and positional parameters are unsupported")); }
                } else if chars.peek() == Some(&'?') {
                    chars.next(); key.push('?');
                } else if chars.peek().is_some_and(|c| *c == '_' || c.is_ascii_alphabetic()) {
                    while chars.peek().is_some_and(|c| *c == '_' || c.is_ascii_alphanumeric()) { key.push(chars.next().unwrap()); }
                } else {
                    if chars.peek().is_some_and(|c| c.is_ascii_digit() || matches!(c, '@' | '*' | '#' | '$' | '!' | '-' | '(')) {
                        return Err(ExpansionError("special parameters and substitutions are unsupported"));
                    }
                    atoms.push(Atom { ch: Some('$'), split: false, quoted }); continue;
                }
                atoms.extend(env.get(&key).unwrap_or("").chars().map(|ch| Atom { ch: Some(ch), split: !quoted, quoted }));
            } else {
                atoms.push(Atom { ch: Some(ch), split: false, quoted });
            }
        }
    }
    Ok(atoms)
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn field_splitting(atoms: Field, ifs: &str) -> Vec<Field> {
    let delim = |a: &Atom| a.split && a.ch.is_some_and(|c| ifs.contains(c));
    let white = |a: &Atom| delim(a) && a.ch.is_some_and(|c| matches!(c, ' ' | '\t' | '\n'));
    let mut result = Vec::new();
    let mut field = Vec::new();
    let mut i = 0;
    while i < atoms.len() {
        if white(&atoms[i]) {
            while i < atoms.len() && white(&atoms[i]) { i += 1; }
            if i < atoms.len() && delim(&atoms[i]) { continue; }
            if !field.is_empty() { result.push(core::mem::take(&mut field)); }
        } else if delim(&atoms[i]) {
            result.push(core::mem::take(&mut field));
            i += 1;
            while i < atoms.len() && white(&atoms[i]) { i += 1; }
        } else { field.push(atoms[i]); i += 1; }
    }
    if !field.is_empty() { result.push(field); }
    result
}

fn pathname_expansion(fields: Vec<Field>) -> Result<Vec<Field>, ExpansionError> {
    if fields.iter().flatten().any(|a| !a.quoted && a.ch.is_some_and(|c| matches!(c, '*' | '?' | '['))) {
        return Err(ExpansionError("pathname expansion is unsupported; quote pattern characters"));
    }
    Ok(fields)
}

fn quote_removal(fields: Vec<Field>) -> Vec<String> {
    fields.into_iter().map(|field| field.into_iter().filter_map(|a| a.ch).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_line_checked;
    fn args(line: &str, env: &ShellEnv) -> Vec<String> {
        let AstNode::Command(cmd) = expand_ast(&parse_line_checked(line).unwrap().unwrap(), env).unwrap() else { panic!() };
        cmd.argv
    }
    #[test]
    fn quote_context_empty_arguments_and_embedded_parameters() {
        let mut env = ShellEnv::new(); env.set("HOME", "/root"); env.set("?", "127");
        assert_eq!(args(r#"echo '$HOME' "$HOME" $HOME "hello world" "" \$HOME pre${HOME}post $?"#, &env),
            ["echo", "$HOME", "/root", "/root", "hello world", "", "$HOME", "pre/rootpost", "127"]);
        assert_eq!(args(r#"echo $UNSET "$UNSET" a""b "x\q" "\$HOME""#, &env), ["echo", "", "ab", "x\\q", "$HOME"]);
    }
    #[test]
    fn splitting_only_unquoted_expansion_regions() {
        let mut env = ShellEnv::new(); env.set("V", "  one\ttwo  ");
        assert_eq!(args(r#"echo $V "$V" pre${V}post"#, &env), ["echo", "one", "two", "  one\ttwo  ", "pre", "one", "two", "post"]);
        env.set("IFS", ": "); env.set("V", " :a::b: ");
        assert_eq!(args(r#"echo $V 'literal: space'"#, &env), ["echo", "", "a", "", "b", "literal: space"]);
        env.set("IFS", ""); assert_eq!(args("echo $V", &env), ["echo", " :a::b: "]);
    }
    #[test]
    fn redirection_does_not_split_and_unsupported_expansions_fail() {
        let mut env = ShellEnv::new(); env.set("V", "a b");
        let AstNode::Command(cmd) = expand_ast(&parse_line_checked("echo > $V").unwrap().unwrap(), &env).unwrap() else { panic!() };
        assert!(matches!(&cmd.redirections[0], Redirection::Open { path, .. } if path == "a b"));
        for line in ["echo ${V:-x}", "echo *.rs", "echo $1", "echo ${V"] {
            assert!(expand_ast(&parse_line_checked(line).unwrap().unwrap(), &env).is_err(), "{line}");
        }
        assert_eq!(args("echo '*.rs'", &env), ["echo", "*.rs"]);
    }
}
