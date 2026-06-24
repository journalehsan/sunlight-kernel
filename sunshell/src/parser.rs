#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::borrow::ToOwned;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub enum AstNode {
    Command(Vec<String>),
    Pipeline(Vec<AstNode>), // Contains a list of Command nodes
}

/// Parse a shell line into an AST, supporting:
/// - Unquoted arguments split on whitespace
/// - Single and double quoted strings (spaces preserved inside quotes)
/// - Pipe operator `|` separating pipeline stages (outside quotes)
///
/// Returns None for blank/empty input.
pub fn parse_line(line: &str) -> Option<AstNode> {
    let mut commands = Vec::new();
    let mut current_args = Vec::new();
    let mut current_token = String::new();

    let mut in_quotes = false;
    let mut quote_char = '\0';
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                if !in_quotes {
                    in_quotes = true;
                    quote_char = c;
                } else if c == quote_char {
                    in_quotes = false;
                } else {
                    current_token.push(c);
                }
            }
            '|' if !in_quotes => {
                if !current_token.is_empty() {
                    current_args.push(current_token.clone());
                    current_token.clear();
                }
                if !current_args.is_empty() {
                    commands.push(AstNode::Command(current_args.clone()));
                    current_args.clear();
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current_token.is_empty() {
                    current_args.push(current_token.clone());
                    current_token.clear();
                }
            }
            _ => current_token.push(c),
        }
    }

    if !current_token.is_empty() {
        current_args.push(current_token);
    }
    if !current_args.is_empty() {
        commands.push(AstNode::Command(current_args));
    }

    match commands.len() {
        0 => None,
        1 => Some(commands.pop().unwrap()),
        _ => Some(AstNode::Pipeline(commands)),
    }
}

/// Backward-compat simple tokenizer (splits on whitespace, no quotes/pipes).
/// Prefer parse_line for new code.
pub fn tokenize(line: &str) -> Vec<String> {
    line.split_ascii_whitespace()
        .map(|s| s.to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn basic_command() {
        match parse_line("echo hello world") {
            Some(AstNode::Command(args)) => assert_eq!(args, vec!["echo", "hello", "world"]),
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn quoted_spaces() {
        match parse_line(r#"echo "hello world" 'single quote'"#) {
            Some(AstNode::Command(args)) => {
                assert_eq!(args, vec!["echo", "hello world", "single quote"]);
            }
            _ => panic!("expected Command with quoted arg"),
        }
    }

    #[test]
    fn simple_pipeline() {
        match parse_line("ls | grep foo") {
            Some(AstNode::Pipeline(stages)) => {
                assert_eq!(stages.len(), 2);
                if let AstNode::Command(a) = &stages[0] {
                    assert_eq!(a, &vec!["ls".to_string()]);
                } else {
                    panic!("expected first stage Command");
                }
                if let AstNode::Command(a) = &stages[1] {
                    assert_eq!(a, &vec!["grep".to_string(), "foo".to_string()]);
                } else {
                    panic!("expected second stage Command");
                }
            }
            _ => panic!("expected Pipeline"),
        }
    }

    #[test]
    fn pipeline_with_quotes() {
        match parse_line(r#"echo "a | b" | cat"#) {
            Some(AstNode::Pipeline(stages)) => {
                assert_eq!(stages.len(), 2);
                if let AstNode::Command(a) = &stages[0] {
                    assert_eq!(a, &vec!["echo".to_string(), "a | b".to_string()]);
                }
            }
            _ => panic!("expected Pipeline preserving quoted |"),
        }
    }

    #[test]
    fn tokenize_still_works() {
        assert_eq!(tokenize("echo hello world"), vec!["echo", "hello", "world"]);
        assert_eq!(tokenize("  cd  /tmp  "), vec!["cd", "/tmp"]);
    }
}
