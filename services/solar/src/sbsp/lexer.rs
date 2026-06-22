//! SBSP Lexer - Streaming Zero-Allocation Iterator
//!
//! Phase 3: Scans .sbsp files and yields SbspToken stream on-demand.
//!
//! Design: Implements Rust's Iterator trait for lazy evaluation.
//! - No buffering: just a pointer into the original string
//! - Zero allocations: all tokens are string slices (&'a str)
//! - Streaming: parse one token at a time as needed
//! - Non-blocking: yields immediately, no I/O waits
//!
//! The lexer hunts for {% and %} delimiters, extracting code blocks and
//! routing remaining text as raw HTML. Perfect for streaming responses to
//! TCP without buffering the entire template.

/// SBSP token (zero-copy)
/// Every token is a slice into the original input string
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbspToken<'a> {
    /// Raw HTML text outside any tags
    Text(&'a str),

    /// Shorthand output: {%| expr %}
    /// The expression will be evaluated and HTML-escaped
    Output(&'a str),

    /// Variable declaration: {% DIM var AS Type [= init] %}
    DimDecl {
        var: &'a str,
        typ: &'a str,
        init: Option<&'a str>,
    },

    /// Conditional: {% IF condition THEN %}
    If(&'a str),
    /// Conditional else: {% ELSE %}
    Else,
    /// Conditional end: {% END IF %}
    EndIf,

    /// Loop: {% FOR var = start TO end %}
    For {
        var: &'a str,
        start: &'a str,
        end: &'a str,
    },
    /// Loop end: {% NEXT %}
    Next,

    /// While loop: {% WHILE condition %}
    While(&'a str),
    /// While loop end: {% WEND %}
    Wend,

    /// Function definition: {% FUNCTION name(params) AS ReturnType %}
    FunctionDef(&'a str),
    /// Function end: {% END FUNCTION %}
    EndFunction,

    /// Return from function: {% RETURN expr %}
    Return(&'a str),

    /// Catch-all for assignments and general expressions
    Expression(&'a str),
}

/// Streaming SBSP lexer (zero-allocation Iterator)
pub struct SbspLexer<'a> {
    /// Remaining unparsed input (advances as we yield tokens)
    remainder: &'a str,
}

impl<'a> SbspLexer<'a> {
    /// Create a new streaming lexer from input string
    ///
    /// # Arguments
    /// * `input` - The .sbsp template as a string slice (lifetime 'a)
    ///
    /// # Zero-Copy Guarantee
    /// The lexer never copies the input. It just maintains a pointer
    /// (`remainder`) and advances it as tokens are yielded.
    pub fn new(input: &'a str) -> Self {
        Self { remainder: input }
    }

    /// Parse the contents of a {% ... %} tag (without the delimiters)
    fn parse_tag_content(tag_body: &'a str) -> SbspToken<'a> {
        let trimmed = tag_body.trim();

        // Handle shorthand output: {%| expr %}
        if let Some(expr) = trimmed.strip_prefix('|') {
            return SbspToken::Output(expr.trim());
        }

        // Handle DIM declarations: {% DIM x AS Integer = 5 %}
        if trimmed.starts_with("DIM ") {
            return Self::parse_dim(trimmed);
        }

        // Handle IF statements: {% IF condition THEN %}
        if let Some(condition) = trimmed.strip_prefix("IF ") {
            if let Some(cond) = condition.strip_suffix(" THEN") {
                return SbspToken::If(cond.trim());
            }
        }

        // Handle control keywords
        if trimmed == "ELSE" {
            return SbspToken::Else;
        }
        if trimmed == "END IF" {
            return SbspToken::EndIf;
        }
        if trimmed == "NEXT" {
            return SbspToken::Next;
        }
        if trimmed == "END FUNCTION" {
            return SbspToken::EndFunction;
        }

        // Handle FOR loops: {% FOR i = 1 TO 10 %}
        if let Some(rest) = trimmed.strip_prefix("FOR ") {
            return Self::parse_for(rest);
        }

        // Handle WHILE loops: {% WHILE condition %}
        if let Some(condition) = trimmed.strip_prefix("WHILE ") {
            return SbspToken::While(condition.trim());
        }

        if trimmed == "WEND" {
            return SbspToken::Wend;
        }

        // Handle FUNCTION definitions: {% FUNCTION name(params) AS Type %}
        if trimmed.starts_with("FUNCTION ") {
            let def_body = &trimmed[9..].trim();
            return SbspToken::FunctionDef(def_body);
        }

        // Handle RETURN: {% RETURN expr %}
        if let Some(expr) = trimmed.strip_prefix("RETURN ") {
            return SbspToken::Return(expr.trim());
        }

        // Fallback for assignments and unparsed expressions
        SbspToken::Expression(trimmed)
    }

    /// Parse DIM declaration: "DIM x AS Integer = 5"
    fn parse_dim(body: &'a str) -> SbspToken<'a> {
        let dim_body = &body[4..]; // Skip "DIM "
        let mut parts = dim_body.splitn(2, " AS ");

        let var = parts.next().unwrap_or("").trim();
        let rest = parts.next().unwrap_or("");

        let mut type_parts = rest.splitn(2, '=');
        let typ = type_parts.next().unwrap_or("").trim();
        let init = type_parts.next().map(|s| s.trim());

        SbspToken::DimDecl { var, typ, init }
    }

    /// Parse FOR loop: "i = 1 TO 10"
    fn parse_for(body: &'a str) -> SbspToken<'a> {
        let mut parts = body.splitn(2, '=');
        let var = parts.next().unwrap_or("").trim();

        let rest = parts.next().unwrap_or("");
        let mut range_parts = rest.splitn(2, " TO ");
        let start = range_parts.next().unwrap_or("").trim();
        let end = range_parts.next().unwrap_or("").trim();

        SbspToken::For { var, start, end }
    }
}

/// Iterator implementation: yields tokens on-demand
impl<'a> Iterator for SbspLexer<'a> {
    type Item = SbspToken<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // Base case: no more input
        if self.remainder.is_empty() {
            return None;
        }

        // Case 1: We're at the start of a tag {% ... %}
        if self.remainder.starts_with("{%") {
            // Hunt for the closing %}
            if let Some(end_idx) = self.remainder.find("%}") {
                // Extract tag content (everything between {% and %})
                let tag_body = &self.remainder[2..end_idx];

                // Advance past the closing %}
                self.remainder = &self.remainder[end_idx + 2..];

                // Parse and return the tag content
                return Some(Self::parse_tag_content(tag_body));
            } else {
                // Malformed: found {% but no %}
                // Consume rest of file as error text to avoid infinite loops
                let bad_text = self.remainder;
                self.remainder = "";
                return Some(SbspToken::Text(bad_text));
            }
        }

        // Case 2: We're at raw HTML, hunt for the next {%
        if let Some(start_idx) = self.remainder.find("{%") {
            // Extract HTML before the tag
            let text = &self.remainder[..start_idx];
            // Advance to the tag
            self.remainder = &self.remainder[start_idx..];
            return Some(SbspToken::Text(text));
        }

        // Case 3: No more tags, the rest is purely HTML
        let text = self.remainder;
        self.remainder = "";
        Some(SbspToken::Text(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_simple_html() {
        let input = "<h1>Hello World</h1>";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(lexer.next(), Some(SbspToken::Text(input)));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_with_output() {
        let input = "<h1>{%| title %}</h1>";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(lexer.next(), Some(SbspToken::Text("<h1>")));
        assert_eq!(lexer.next(), Some(SbspToken::Output("title")));
        assert_eq!(lexer.next(), Some(SbspToken::Text("</h1>")));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_dim_declaration() {
        let input = "{% DIM x AS Integer = 5 %}";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(
            lexer.next(),
            Some(SbspToken::DimDecl {
                var: "x",
                typ: "Integer",
                init: Some("5")
            })
        );
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_dim_no_init() {
        let input = "{% DIM count AS Integer %}";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(
            lexer.next(),
            Some(SbspToken::DimDecl {
                var: "count",
                typ: "Integer",
                init: None
            })
        );
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_if_statement() {
        let input = "{% IF x > 5 THEN %}<p>Yes</p>{% END IF %}";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(lexer.next(), Some(SbspToken::If("x > 5")));
        assert_eq!(lexer.next(), Some(SbspToken::Text("<p>Yes</p>")));
        assert_eq!(lexer.next(), Some(SbspToken::EndIf));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_for_loop() {
        let input = "{% FOR i = 1 TO 10 %}<li>{%| i %}</li>{% NEXT %}";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(
            lexer.next(),
            Some(SbspToken::For {
                var: "i",
                start: "1",
                end: "10"
            })
        );
        assert_eq!(lexer.next(), Some(SbspToken::Text("<li>")));
        assert_eq!(lexer.next(), Some(SbspToken::Output("i")));
        assert_eq!(lexer.next(), Some(SbspToken::Text("</li>")));
        assert_eq!(lexer.next(), Some(SbspToken::Next));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_complex_flow() {
        let input = "<h1>{%| title %}</h1>\n{% DIM x AS Integer = 5 %}<p>{% IF x > 0 THEN %}Positive{% END IF %}</p>";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(lexer.next(), Some(SbspToken::Text("<h1>")));
        assert_eq!(lexer.next(), Some(SbspToken::Output("title")));
        assert_eq!(lexer.next(), Some(SbspToken::Text("</h1>\n")));
        assert_eq!(
            lexer.next(),
            Some(SbspToken::DimDecl {
                var: "x",
                typ: "Integer",
                init: Some("5")
            })
        );
        assert_eq!(lexer.next(), Some(SbspToken::Text("<p>")));
        assert_eq!(lexer.next(), Some(SbspToken::If("x > 0")));
        assert_eq!(lexer.next(), Some(SbspToken::Text("Positive")));
        assert_eq!(lexer.next(), Some(SbspToken::EndIf));
        assert_eq!(lexer.next(), Some(SbspToken::Text("</p>")));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_function_definition() {
        let input = "{% FUNCTION add(a, b) AS Integer %}{% RETURN a + b %}{% END FUNCTION %}";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(
            lexer.next(),
            Some(SbspToken::FunctionDef("add(a, b) AS Integer"))
        );
        assert_eq!(lexer.next(), Some(SbspToken::Return("a + b")));
        assert_eq!(lexer.next(), Some(SbspToken::EndFunction));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_while_loop() {
        let input = "{% WHILE count > 0 %}<li>Count</li>{% WEND %}";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(lexer.next(), Some(SbspToken::While("count > 0")));
        assert_eq!(lexer.next(), Some(SbspToken::Text("<li>Count</li>")));
        assert_eq!(lexer.next(), Some(SbspToken::Wend));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_malformed_tag() {
        let input = "Text {% UNCLOSED TAG";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(lexer.next(), Some(SbspToken::Text("Text ")));
        assert_eq!(lexer.next(), Some(SbspToken::Text("{% UNCLOSED TAG")));
        assert_eq!(lexer.next(), None);
    }

    #[test]
    fn test_lexer_multiple_adjacent_tags() {
        let input = "{%| x %}{%| y %}";
        let mut lexer = SbspLexer::new(input);

        assert_eq!(lexer.next(), Some(SbspToken::Output("x")));
        assert_eq!(lexer.next(), Some(SbspToken::Output("y")));
        assert_eq!(lexer.next(), None);
    }
}
