//! Bounded recursive-descent shell grammar over the shared quote-aware lexer.
use crate::parser::{lex_spanned, Quote, Redirection, SimpleCommand, Token, Word};
use alloc::{boxed::Box, string::String, vec::Vec};

pub const MAX_SOURCE: usize = 65536;
pub const MAX_NESTING: usize = 32;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub kind: CommandKind,
    pub redirections: Vec<Redirection>,
    pub offset: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandKind {
    SimpleCommand(SimpleCommand),
    Pipeline(Vec<Command>),
    AndOrList {
        first: Box<Command>,
        rest: Vec<(bool, Command)>,
    },
    SequentialList(Vec<Command>),
    IfCommand {
        branches: Vec<(Command, Command)>,
        otherwise: Option<Box<Command>>,
    },
    ForCommand {
        name: String,
        words: Option<Vec<Word>>,
        body: Box<Command>,
    },
    WhileCommand {
        condition: Box<Command>,
        body: Box<Command>,
    },
    UntilCommand {
        condition: Box<Command>,
        body: Box<Command>,
    },
    CaseCommand {
        word: Word,
        arms: Vec<(Vec<Word>, Command)>,
    },
    FunctionDefinition {
        name: String,
        body: Box<Command>,
    },
    BraceGroup(Box<Command>),
    Subshell(Box<Command>),
    NegatedPipeline(Box<Command>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: &'static str,
    pub offset: usize,
    pub incomplete: bool,
}
impl SyntaxError {
    pub fn location(&self, source: &str) -> (usize, usize) {
        let prefix = &source[..self.offset.min(source.len())];
        (
            prefix.bytes().filter(|b| *b == b'\n').count() + 1,
            prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        )
    }
}
pub fn parse(source: &str) -> Result<Command, SyntaxError> {
    let tokens = lex_spanned(source).map_err(|e| SyntaxError {
        message: e.message,
        offset: e.offset,
        incomplete: e.incomplete,
    })?;
    let mut parser = Parser {
        tokens,
        index: 0,
        depth: 0,
        end: source.len(),
    };
    let tree = parser.list(&[], true)?;
    if parser.peek().is_some() {
        return Err(parser.error("unexpected token"));
    }
    Ok(tree)
}
/// The grammar decides whether a right parenthesis closes the substitution,
/// a subshell, a function header or a case pattern. Incomplete case arms cause
/// the lexer to continue; no parenthesis counter is used.
pub(crate) fn substitution_tree(
    tokens: &[(Token, usize)],
    end: usize,
) -> Result<Option<Command>, SyntaxError> {
    let mut parser = Parser {
        tokens: tokens.to_vec(),
        index: 0,
        depth: 0,
        end,
    };
    let result = parser.list(&[], true);
    match result {
        Ok(tree)
            if matches!(parser.peek(), Some(Token::RightParen))
                && parser.index + 1 == tokens.len() =>
        {
            Ok(Some(tree))
        }
        Ok(_) => Ok(None),
        Err(e) if e.incomplete => Ok(None),
        Err(e) => Err(e),
    }
}
struct Parser {
    tokens: Vec<(Token, usize)>,
    index: usize,
    depth: usize,
    end: usize,
}
impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index).map(|t| &t.0)
    }
    fn offset(&self) -> usize {
        self.tokens.get(self.index).map(|t| t.1).unwrap_or(self.end)
    }
    fn error(&self, message: &'static str) -> SyntaxError {
        SyntaxError {
            message,
            offset: self.offset(),
            incomplete: self.peek().is_none(),
        }
    }
    fn word_is(&self, value: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w.parts.len() == 1 && w.parts[0].quote == Quote::Unquoted && w.parts[0].text == value)
    }
    fn eat_word(&mut self, value: &str) -> bool {
        if self.word_is(value) {
            self.index += 1;
            true
        } else {
            false
        }
    }
    fn expect_word(&mut self, value: &str) -> Result<(), SyntaxError> {
        if self.eat_word(value) {
            Ok(())
        } else {
            Err(self.error("expected shell keyword"))
        }
    }
    fn separators(&mut self) {
        while matches!(self.peek(), Some(Token::Separator)) {
            self.index += 1;
        }
    }
    fn take_word(&mut self) -> Result<Word, SyntaxError> {
        match self.peek().cloned() {
            Some(Token::Word(w)) => {
                self.index += 1;
                Ok(w)
            }
            _ => Err(self.error("expected word")),
        }
    }
    fn list(&mut self, stops: &[&str], empty: bool) -> Result<Command, SyntaxError> {
        let offset = self.offset();
        let mut commands = Vec::new();
        self.separators();
        while self.peek().is_some()
            && !stops.iter().any(|s| self.word_is(s))
            && !matches!(self.peek(), Some(Token::RightParen | Token::DoubleSemi))
        {
            commands.push(self.and_or()?);
            if matches!(self.peek(), Some(Token::Separator)) {
                self.separators();
            } else if self.peek().is_none()
                || matches!(self.peek(), Some(Token::RightParen | Token::DoubleSemi))
            {
                break;
            } else {
                return Err(self.error("expected semicolon or newline"));
            }
        }
        if commands.is_empty() && !empty {
            return Err(self.error("expected command list"));
        }
        Ok(Command {
            kind: CommandKind::SequentialList(commands),
            redirections: Vec::new(),
            offset,
        })
    }
    fn and_or(&mut self) -> Result<Command, SyntaxError> {
        let first = self.pipeline()?;
        let offset = first.offset;
        let mut rest = Vec::new();
        while matches!(self.peek(), Some(Token::And | Token::Or)) {
            let and = matches!(self.peek(), Some(Token::And));
            self.index += 1;
            self.separators();
            rest.push((and, self.pipeline()?));
        }
        if rest.is_empty() {
            Ok(first)
        } else {
            Ok(Command {
                kind: CommandKind::AndOrList {
                    first: Box::new(first),
                    rest,
                },
                redirections: Vec::new(),
                offset,
            })
        }
    }
    fn pipeline(&mut self) -> Result<Command, SyntaxError> {
        let offset = self.offset();
        let negated = self.eat_word("!");
        let mut commands = alloc::vec![self.command()?];
        while matches!(self.peek(), Some(Token::Pipe)) {
            self.index += 1;
            self.separators();
            commands.push(self.command()?);
        }
        let mut tree = if commands.len() == 1 {
            commands.pop().unwrap()
        } else {
            Command {
                kind: CommandKind::Pipeline(commands),
                redirections: Vec::new(),
                offset,
            }
        };
        if negated {
            tree = Command {
                kind: CommandKind::NegatedPipeline(Box::new(tree)),
                redirections: Vec::new(),
                offset,
            };
        }
        Ok(tree)
    }
    fn command(&mut self) -> Result<Command, SyntaxError> {
        if self.depth >= MAX_NESTING {
            return Err(self.error("syntax nesting exceeds 32 levels"));
        }
        self.depth += 1;
        let result = self.command_inner();
        self.depth -= 1;
        result
    }
    fn command_inner(&mut self) -> Result<Command, SyntaxError> {
        let offset = self.offset();
        let kind = if self.eat_word("if") {
            let mut branches = Vec::new();
            loop {
                let condition = self.list(&["then"], false)?;
                self.expect_word("then")?;
                let body = self.list(&["elif", "else", "fi"], false)?;
                branches.push((condition, body));
                if !self.eat_word("elif") {
                    break;
                }
            }
            let otherwise = if self.eat_word("else") {
                Some(Box::new(self.list(&["fi"], false)?))
            } else {
                None
            };
            self.expect_word("fi")?;
            CommandKind::IfCommand {
                branches,
                otherwise,
            }
        } else if self.eat_word("for") {
            let word = self.take_word()?;
            let name = plain_name(&word).ok_or_else(|| self.error("invalid for variable"))?;
            let separated = matches!(self.peek(), Some(Token::Separator));
            self.separators();
            let words = if self.eat_word("in") {
                let mut words = Vec::new();
                while matches!(self.peek(), Some(Token::Word(_))) {
                    words.push(self.take_word()?);
                }
                if !matches!(self.peek(), Some(Token::Separator)) {
                    return Err(self.error("expected separator after for words"));
                }
                self.separators();
                Some(words)
            } else {
                if !separated {
                    return Err(self.error("expected separator before for do-group"));
                }
                None
            };
            self.expect_word("do")?;
            let body = Box::new(self.list(&["done"], false)?);
            self.expect_word("done")?;
            CommandKind::ForCommand { name, words, body }
        } else if self.word_is("while") || self.word_is("until") {
            let until = self.eat_word("until");
            if !until {
                self.expect_word("while")?;
            }
            let condition = Box::new(self.list(&["do"], false)?);
            self.expect_word("do")?;
            let body = Box::new(self.list(&["done"], false)?);
            self.expect_word("done")?;
            if until {
                CommandKind::UntilCommand { condition, body }
            } else {
                CommandKind::WhileCommand { condition, body }
            }
        } else if self.eat_word("case") {
            let word = self.take_word()?;
            self.separators();
            self.expect_word("in")?;
            self.separators();
            let mut arms = Vec::new();
            while !self.word_is("esac") {
                if matches!(self.peek(), Some(Token::LeftParen)) {
                    self.index += 1;
                }
                let mut patterns = alloc::vec![self.take_word()?];
                while matches!(self.peek(), Some(Token::Pipe)) {
                    self.index += 1;
                    patterns.push(self.take_word()?);
                }
                if !matches!(self.peek(), Some(Token::RightParen)) {
                    return Err(self.error("expected case pattern closing parenthesis"));
                }
                self.index += 1;
                let body = self.list(&["esac"], true)?;
                arms.push((patterns, body));
                if matches!(self.peek(), Some(Token::DoubleSemi)) {
                    self.index += 1;
                    self.separators();
                } else if !self.word_is("esac") {
                    return Err(self.error("expected case arm terminator"));
                }
            }
            self.expect_word("esac")?;
            CommandKind::CaseCommand { word, arms }
        } else if self.eat_word("{") {
            let body = Box::new(self.list(&["}"], false)?);
            self.expect_word("}")?;
            CommandKind::BraceGroup(body)
        } else if matches!(self.peek(), Some(Token::LeftParen)) {
            self.index += 1;
            let body = Box::new(self.list(&[], false)?);
            if !matches!(self.peek(), Some(Token::RightParen)) {
                return Err(self.error("expected closing parenthesis"));
            }
            self.index += 1;
            CommandKind::Subshell(body)
        } else if matches!(self.peek(), Some(Token::Word(_)))
            && matches!(
                self.tokens.get(self.index + 1).map(|t| &t.0),
                Some(Token::LeftParen)
            )
        {
            let word = self.take_word()?;
            let name = plain_name(&word).ok_or_else(|| self.error("invalid function name"))?;
            self.index += 1;
            if !matches!(self.peek(), Some(Token::RightParen)) {
                return Err(self.error("expected empty function parentheses"));
            }
            self.index += 1;
            self.separators();
            let body = Box::new(self.command()?);
            if matches!(body.kind, CommandKind::SimpleCommand(_)) {
                return Err(self.error("function requires a compound body"));
            }
            CommandKind::FunctionDefinition { name, body }
        } else {
            if ["then", "else", "elif", "fi", "do", "done", "esac", "}"]
                .iter()
                .any(|s| self.word_is(s))
            {
                return Err(self.error("unexpected reserved word"));
            }
            let mut argv = Vec::new();
            let mut redirections = Vec::new();
            loop {
                match self.peek() {
                    Some(Token::Word(_)) => argv.push(self.take_word()?),
                    Some(Token::Redirect(..) | Token::Dup(..)) => {
                        redirections.push(self.redirect()?)
                    }
                    _ => break,
                }
            }
            if argv.is_empty() && redirections.is_empty() {
                return Err(self.error("expected command"));
            }
            CommandKind::SimpleCommand(SimpleCommand { argv, redirections })
        };
        let mut redirections = Vec::new();
        while matches!(self.peek(), Some(Token::Redirect(..) | Token::Dup(..))) {
            redirections.push(self.redirect()?);
        }
        Ok(Command {
            kind,
            redirections,
            offset,
        })
    }
    fn redirect(&mut self) -> Result<Redirection, SyntaxError> {
        let token = self.peek().cloned().unwrap();
        self.index += 1;
        Ok(match token {
            Token::Redirect(mode, fd) => Redirection::Open {
                fd,
                mode,
                path: self.take_word()?,
            },
            Token::Dup(fd, source) => Redirection::Dup { fd, source },
            _ => unreachable!(),
        })
    }
}
fn plain_name(word: &Word) -> Option<String> {
    if word.parts.len() == 1
        && word.parts[0].quote == Quote::Unquoted
        && crate::expansion::valid_name(&word.parts[0].text)
    {
        Some(word.parts[0].text.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_grammar() {
        for script in [
            "if true; then if false; then :; else :; fi; elif false; then :; fi",
            "for x in a b; do while false; do :; done; done",
            "f() { case $1 in a|b) : ;; *) : ;; esac; }; (f a) && ! false",
        ] {
            assert!(parse(script).is_ok(), "{script}");
        }
    }
    #[test]
    fn errors_and_positions() {
        let e = parse("echo x\nif true; then\n").unwrap_err();
        assert!(e.incomplete);
        assert_eq!(e.location("echo x\nif true; then\n"), (3, 1));
        for source in [
            "if; then :; fi",
            "for 2x in a; do :; done",
            "for x do :; done",
            "echo a && || echo b",
            "{ : }",
            "echo >",
            "cat <<EOF",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
        assert!(parse(&"(".repeat(MAX_NESTING + 1)).is_err());
    }
    #[test]
    fn substitution_boundaries_are_decided_by_the_grammar() {
        for source in [
            "echo \"$(echo \"$(echo nested)\")\"",
            "echo $(case x in x) echo ')';; *) :;; esac)",
            "echo $(f() { echo ')'; }; f)",
            "echo $( (echo sub); echo end # ) comment\n)",
            "echo '$(not parsed' \\\\$escaped",
        ] {
            assert!(parse(source).is_ok(), "{source}");
        }
        let source = "echo first\necho $(echo |)";
        let error = parse(source).unwrap_err();
        assert_eq!(error.location(source), (2, 14));
        assert!(!error.incomplete);
        assert!(parse("echo $(echo").unwrap_err().incomplete);
        assert!(parse("echo $((1+2))").is_ok());
        let e = parse("echo x\necho $((1+2)").unwrap_err();
        assert_eq!(e.location("echo x\necho $((1+2)"), (2, 12));
        assert!(!e.incomplete);
        let source = format!("{}true{}", "echo $(".repeat(33), ")".repeat(33));
        assert!(parse(&source).is_err());
    }
}
