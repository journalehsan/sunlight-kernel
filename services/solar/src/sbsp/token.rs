//! SBSP Token Types
//!
//! Lexical tokens produced by the SBSP lexer.
//! Phase 3: Character-by-character tokenization of .sbsp files

#[derive(Debug, Clone, PartialEq)]
pub enum SbspToken {
    /// Plain HTML text outside {% %} blocks
    Text(heapless::String<512>),

    /// Variable/expression output: {%| expr %}
    Expr(heapless::String<256>),

    /// Variable declaration: {% DIM var AS Type [= init] %}
    DimDecl {
        var: heapless::String<64>,
        typ: heapless::String<64>,
        init: Option<heapless::String<256>>,
    },

    /// Conditional block start: {% IF condition THEN %}
    If(heapless::String<256>),

    /// Conditional block else: {% ELSE %}
    Else,

    /// Conditional block end: {% END IF %}
    EndIf,

    /// Loop start: {% FOR var = start TO end %}
    For {
        var: heapless::String<64>,
        start: heapless::String<64>,
        end: heapless::String<64>,
    },

    /// Loop end: {% NEXT %}
    Next,

    /// While loop start: {% WHILE condition %}
    While(heapless::String<256>),

    /// While loop end: {% WEND %}
    Wend,

    /// Function definition start: {% FUNCTION name(params) AS ReturnType %}
    FunctionDef {
        name: heapless::String<64>,
        params: heapless::Vec<(heapless::String<64>, heapless::String<64>), 8>,
        return_type: heapless::String<64>,
    },

    /// Return from function: {% RETURN expr %}
    Return(heapless::String<256>),

    /// Function definition end: {% END FUNCTION %}
    EndFunction,

    /// Variable assignment: {% var = expr %}
    Assignment {
        var: heapless::String<64>,
        expr: heapless::String<256>,
    },

    /// End of input
    Eof,
}

impl SbspToken {
    pub fn is_block_start(&self) -> bool {
        matches!(self, SbspToken::If(_) | SbspToken::For { .. } | SbspToken::While(_) | SbspToken::FunctionDef { .. })
    }

    pub fn is_block_end(&self) -> bool {
        matches!(self, SbspToken::EndIf | SbspToken::Next | SbspToken::Wend | SbspToken::EndFunction)
    }
}
