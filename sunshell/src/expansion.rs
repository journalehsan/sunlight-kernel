//! Ordered expansion stages shared by native and host execution.
extern crate alloc;
use crate::parser::{AstNode, Quote, Redirection, SimpleCommand, Word};
use crate::shellenv::ShellEnv;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug)]
pub enum ExpansionError {
    Expansion(&'static str),
    Arithmetic(&'static str),
}
impl ExpansionError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Expansion(message) | Self::Arithmetic(message) => message,
        }
    }
}

// None is a quoted empty anchor; it survives splitting and is removed last.
#[derive(Clone, Copy)]
struct Atom {
    ch: Option<char>,
    split: bool,
    quoted: bool,
    substituted: bool,
}
type Field = Vec<Atom>;
fn field_bytes(fields: &[Field]) -> usize {
    fields
        .iter()
        .flatten()
        .filter_map(|a| a.ch)
        .map(char::len_utf8)
        .sum()
}

pub fn expand_ast(ast: &AstNode, env: &ShellEnv) -> Result<AstNode<String>, ExpansionError> {
    fn command(
        cmd: &SimpleCommand,
        env: &ShellEnv,
    ) -> Result<SimpleCommand<String>, ExpansionError> {
        let mut argv = Vec::new();
        for word in &cmd.argv {
            argv.extend(expand_word(word, env, true)?);
        }
        let mut redirections = Vec::new();
        for redir in &cmd.redirections {
            redirections.push(match redir {
                Redirection::Dup { fd, source } => Redirection::Dup {
                    fd: *fd,
                    source: *source,
                },
                Redirection::Open { fd, mode, path } => {
                    // POSIX redirection words are not subject to field splitting.
                    let mut fields = expand_word(path, env, false)?;
                    Redirection::Open {
                        fd: *fd,
                        mode: *mode,
                        path: fields.remove(0),
                    }
                }
            });
        }
        Ok(SimpleCommand { argv, redirections })
    }
    Ok(match ast {
        AstNode::Command(cmd) => AstNode::Command(command(cmd, env)?),
        AstNode::Pipeline(cmds) => AstNode::Pipeline(
            cmds.iter()
                .map(|cmd| command(cmd, env))
                .collect::<Result<_, _>>()?,
        ),
    })
}

pub fn expand_word(
    word: &Word,
    env: &ShellEnv,
    split: bool,
) -> Result<Vec<String>, ExpansionError> {
    let segments = parameter_expansion(word, env)?;
    let mut fields = Vec::new();
    for atoms in segments {
        if split {
            fields.extend(field_splitting(atoms, env.get("IFS").unwrap_or(" \t\n")));
        } else {
            fields.push(atoms);
        }
    }
    let fields = if split {
        pathname_expansion(fields)?
    } else {
        fields
    };
    Ok(quote_removal(fields))
}

/// Expand a word while allowing the interpreter to evaluate `$(...)` nodes.
/// The callback owns process creation/capture; this module only preserves
/// quote provenance and applies field splitting exactly once.
pub fn expand_word_with<F>(
    word: &Word,
    env: &mut ShellEnv,
    split: bool,
    mut substitute: F,
) -> Result<Vec<String>, ExpansionError>
where
    F: FnMut(&crate::script_parser::Command, &ShellEnv) -> Result<String, ExpansionError>,
{
    let segments = parameter_expansion_with(word, env, &mut substitute)?;
    let mut fields = Vec::new();
    for atoms in segments {
        if split {
            fields.extend(field_splitting(atoms, env.get("IFS").unwrap_or(" \t\n")));
        } else {
            fields.push(atoms);
        }
    }
    let fields = if split {
        pathname_expansion(fields)?
    } else {
        fields
    };
    Ok(quote_removal(fields))
}

fn parameter_expansion(word: &Word, env: &ShellEnv) -> Result<Vec<Field>, ExpansionError> {
    parameter_expansion_with(word, &mut env.clone(), &mut |_, _| {
        Err(ExpansionError::Expansion(
            "command substitution requires an execution runtime",
        ))
    })
}

fn parameter_expansion_with<F>(
    word: &Word,
    env: &mut ShellEnv,
    substitute: &mut F,
) -> Result<Vec<Field>, ExpansionError>
where
    F: FnMut(&crate::script_parser::Command, &ShellEnv) -> Result<String, ExpansionError>,
{
    let mut fields: Vec<Field> = alloc::vec![Vec::new()];
    let mut only_empty_at = true;
    for part in &word.parts {
        let quoted = part.quote != Quote::Unquoted;
        let expandable = matches!(part.quote, Quote::Unquoted | Quote::Double);
        let at_only = expandable && matches!(part.text.as_str(), "$@" | "${@}");
        if quoted && !at_only {
            fields.last_mut().unwrap().push(Atom {
                ch: None,
                split: false,
                quoted: true,
                substituted: false,
            });
            only_empty_at = false;
        }
        if part.substitution.is_some() || part.arithmetic.is_some() {
            let value = if let Some(command) = &part.substitution {
                substitute(command, env)?
            } else {
                let expression = part.arithmetic.as_ref().unwrap();
                let fields = parameter_expansion_with(expression, env, substitute)?;
                let mut values = quote_removal(fields);
                let text = if values.len() <= 1 {
                    values.pop().unwrap_or_default()
                } else {
                    // Arithmetic is scalar: positional fields use the first
                    // IFS character, as for the reference shell's arithmetic $@.
                    let separator = env
                        .get("IFS")
                        .unwrap_or(" ")
                        .chars()
                        .next()
                        .map(|ch| ch.to_string())
                        .unwrap_or_default();
                    values.join(&separator)
                };
                crate::arithmetic::evaluate(&text, env)?.to_string()
            };
            if field_bytes(&fields) + value.len() > 16384 {
                return Err(ExpansionError::Expansion(
                    "expanded word exceeds 16 KiB limit",
                ));
            }
            fields
                .last_mut()
                .unwrap()
                .extend(value.chars().map(|ch| Atom {
                    ch: Some(ch),
                    split: !quoted,
                    quoted,
                    substituted: true,
                }));
            only_empty_at = false;
            continue;
        }
        let mut chars = part.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if expandable && ch == '`' {
                return Err(ExpansionError::Expansion(
                    "command substitution is unsupported",
                ));
            }
            if expandable && ch == '$' {
                let mut key = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    while chars.peek().is_some_and(|c| *c != '}') {
                        key.push(chars.next().unwrap());
                    }
                    if chars.next() != Some('}') {
                        return Err(ExpansionError::Expansion(
                            "unterminated parameter expansion",
                        ));
                    }
                    if !valid_name(&key)
                        && !matches!(
                            key.as_str(),
                            "?" | "@"
                                | "*"
                                | "#"
                                | "$"
                                | "0"
                                | "1"
                                | "2"
                                | "3"
                                | "4"
                                | "5"
                                | "6"
                                | "7"
                                | "8"
                                | "9"
                        )
                    {
                        return Err(ExpansionError::Expansion(
                            "parameter operators are unsupported",
                        ));
                    }
                } else if chars
                    .peek()
                    .is_some_and(|c| c.is_ascii_digit() || matches!(c, '?' | '@' | '*' | '#' | '$'))
                {
                    key.push(chars.next().unwrap());
                } else if chars
                    .peek()
                    .is_some_and(|c| *c == '_' || c.is_ascii_alphabetic())
                {
                    while chars
                        .peek()
                        .is_some_and(|c| *c == '_' || c.is_ascii_alphanumeric())
                    {
                        key.push(chars.next().unwrap());
                    }
                } else {
                    if chars.peek().is_some_and(|c| matches!(c, '!' | '-' | '(')) {
                        return Err(ExpansionError::Expansion(
                            "substitutions and shell-option parameters are unsupported",
                        ));
                    }
                    fields.last_mut().unwrap().push(Atom {
                        ch: Some('$'),
                        split: false,
                        quoted,
                        substituted: false,
                    });
                    only_empty_at = false;
                    continue;
                }
                if key == "@" {
                    for (i, value) in env.positional.iter().enumerate() {
                        if i > 0 {
                            fields.push(Vec::new());
                        }
                        let fields_size_before = field_bytes(&fields);
                        let atoms = fields.last_mut().unwrap();
                        if quoted {
                            atoms.push(Atom {
                                ch: None,
                                split: false,
                                quoted,
                                substituted: false,
                            });
                        }
                        if fields_size_before + value.len() > 16384 {
                            return Err(ExpansionError::Expansion(
                                "expanded word exceeds 16 KiB limit",
                            ));
                        }
                        atoms.extend(value.chars().map(|ch| Atom {
                            ch: Some(ch),
                            split: !quoted,
                            quoted,
                            substituted: false,
                        }));
                    }
                    if !env.positional.is_empty() {
                        only_empty_at = false;
                    }
                } else {
                    only_empty_at = false;
                    let value = match key.as_str() {
                        "0" => env.arg0.clone(),
                        "#" => alloc::format!("{}", env.positional.len()),
                        "$" => alloc::format!("{}", env.pid),
                        "*" => env.positional.join(
                            &env.get("IFS")
                                .unwrap_or(" ")
                                .chars()
                                .next()
                                .map(|c| alloc::format!("{c}"))
                                .unwrap_or_default(),
                        ),
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => env
                            .positional
                            .get(key.as_bytes()[0] as usize - b'1' as usize)
                            .cloned()
                            .unwrap_or_default(),
                        _ => String::from(env.get(&key).unwrap_or("")),
                    };
                    if field_bytes(&fields) + value.len() > 16384 {
                        return Err(ExpansionError::Expansion(
                            "expanded word exceeds 16 KiB limit",
                        ));
                    }
                    fields
                        .last_mut()
                        .unwrap()
                        .extend(value.chars().map(|ch| Atom {
                            ch: Some(ch),
                            split: !quoted,
                            quoted,
                            substituted: false,
                        }));
                }
            } else {
                only_empty_at = false;
                fields.last_mut().unwrap().push(Atom {
                    ch: Some(ch),
                    split: false,
                    quoted,
                    substituted: false,
                });
            }
        }
    }
    if only_empty_at {
        fields.clear();
    }
    if field_bytes(&fields) > 16384 {
        return Err(ExpansionError::Expansion(
            "expanded word exceeds 16 KiB limit",
        ));
    }
    Ok(fields)
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
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
            while i < atoms.len() && white(&atoms[i]) {
                i += 1;
            }
            if i < atoms.len() && delim(&atoms[i]) {
                continue;
            }
            if !field.is_empty() {
                result.push(core::mem::take(&mut field));
            }
        } else if delim(&atoms[i]) {
            result.push(core::mem::take(&mut field));
            i += 1;
            while i < atoms.len() && white(&atoms[i]) {
                i += 1;
            }
        } else {
            field.push(atoms[i]);
            i += 1;
        }
    }
    if !field.is_empty() {
        result.push(field);
    }
    result
}

fn pathname_expansion(fields: Vec<Field>) -> Result<Vec<Field>, ExpansionError> {
    if fields
        .iter()
        .flatten()
        .any(|a| !a.quoted && !a.substituted && a.ch.is_some_and(|c| matches!(c, '*' | '?' | '[')))
    {
        return Err(ExpansionError::Expansion(
            "pathname expansion is unsupported; quote pattern characters",
        ));
    }
    Ok(fields)
}

fn quote_removal(fields: Vec<Field>) -> Vec<String> {
    fields
        .into_iter()
        .map(|field| field.into_iter().filter_map(|a| a.ch).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_line_checked;
    fn args(line: &str, env: &ShellEnv) -> Vec<String> {
        let AstNode::Command(cmd) =
            expand_ast(&parse_line_checked(line).unwrap().unwrap(), env).unwrap()
        else {
            panic!()
        };
        cmd.argv
    }
    #[test]
    fn quote_context_empty_arguments_and_embedded_parameters() {
        let mut env = ShellEnv::new();
        env.set("HOME", "/root");
        env.set("?", "127");
        assert_eq!(
            args(
                r#"echo '$HOME' "$HOME" $HOME "hello world" "" \$HOME pre${HOME}post $?"#,
                &env
            ),
            [
                "echo",
                "$HOME",
                "/root",
                "/root",
                "hello world",
                "",
                "$HOME",
                "pre/rootpost",
                "127"
            ]
        );
        assert_eq!(
            args(r#"echo $UNSET "$UNSET" a""b "x\q" "\$HOME""#, &env),
            ["echo", "", "ab", "x\\q", "$HOME"]
        );
    }
    #[test]
    fn splitting_only_unquoted_expansion_regions() {
        let mut env = ShellEnv::new();
        env.set("V", "  one\ttwo  ");
        assert_eq!(
            args(r#"echo $V "$V" pre${V}post"#, &env),
            [
                "echo",
                "one",
                "two",
                "  one\ttwo  ",
                "pre",
                "one",
                "two",
                "post"
            ]
        );
        env.set("IFS", ": ");
        env.set("V", " :a::b: ");
        assert_eq!(
            args(r#"echo $V 'literal: space'"#, &env),
            ["echo", "", "a", "", "b", "literal: space"]
        );
        env.set("IFS", "");
        assert_eq!(args("echo $V", &env), ["echo", " :a::b: "]);
    }
    #[test]
    fn positional_fields_empty_at_and_ifs_joining() {
        let mut env = ShellEnv::new();
        env.arg0 = String::from("file.sh");
        env.positional = alloc::vec![String::from("a b"), String::new(), String::from("c")];
        assert_eq!(
            args(r#"echo pre"$@"post "$@" $@ "$*" $# $0 $9"#, &env),
            [
                "echo", "prea b", "", "cpost", "a b", "", "c", "a", "b", "c", "a b  c", "3",
                "file.sh"
            ]
        );
        env.set("IFS", "");
        assert_eq!(args(r#"echo "$*" $@"#, &env), ["echo", "a bc", "a b", "c"]);
        env.positional.clear();
        assert_eq!(
            args(r#"echo "$@" pre"$@"post "$*" "$@"''"#, &env),
            ["echo", "prepost", "", ""]
        );
    }

    #[test]
    fn assignment_scalars_and_expansion_budget() {
        let mut env = ShellEnv::new();
        env.set("V", "a b *");
        assert_eq!(
            expand_word(&Word::from("$V"), &env, false).unwrap(),
            ["a b *"]
        );
        env.set("V", &"x".repeat(10000));
        assert!(expand_word(&Word::from("${V}${V}"), &env, false).is_err());
    }

    #[test]
    fn redirection_does_not_split_and_unsupported_expansions_fail() {
        let mut env = ShellEnv::new();
        env.set("V", "a b");
        let AstNode::Command(cmd) =
            expand_ast(&parse_line_checked("echo > $V").unwrap().unwrap(), &env).unwrap()
        else {
            panic!()
        };
        assert!(matches!(&cmd.redirections[0], Redirection::Open { path, .. } if path == "a b"));
        for line in ["echo ${V:-x}", "echo *.rs", "echo ${V"] {
            assert!(
                expand_ast(&parse_line_checked(line).unwrap().unwrap(), &env).is_err(),
                "{line}"
            );
        }
        assert_eq!(args("echo '*.rs'", &env), ["echo", "*.rs"]);
    }
}

/// Pattern expansion retains quote masks; quoted wildcards remain literal.
pub fn case_pattern(word: &Word, env: &ShellEnv) -> Result<Vec<(char, bool)>, ExpansionError> {
    Ok(parameter_expansion(word, env)?
        .into_iter()
        .flatten()
        .filter_map(|a| a.ch.map(|c| (c, a.quoted)))
        .collect())
}

pub fn case_pattern_with<F>(
    word: &Word,
    env: &mut ShellEnv,
    mut substitute: F,
) -> Result<Vec<(char, bool)>, ExpansionError>
where
    F: FnMut(&crate::script_parser::Command, &ShellEnv) -> Result<String, ExpansionError>,
{
    Ok(parameter_expansion_with(word, env, &mut substitute)?
        .into_iter()
        .flatten()
        .filter_map(|a| a.ch.map(|c| (c, a.quoted)))
        .collect())
}
