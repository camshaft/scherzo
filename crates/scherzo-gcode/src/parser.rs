use crate::lexer::{LexError, Token, TokenKind, Value, lex};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Statement {
    pub line: usize,
    pub raw: String,
    pub words: Vec<Word>,
    pub comment: Option<String>,
    pub checksum: Option<u8>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Word {
    pub letter: Option<char>,
    pub name: Option<String>,
    pub value: Option<Value>,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error(transparent)]
    Lex(#[from] LexError),

    #[error("multiple comments on line {line}")]
    MultipleComments { line: usize },

    #[error("multiple checksums on line {line}")]
    MultipleChecksums { line: usize },
}

/// Parse G-code from a string using the lexer.
pub fn parse(input: &str) -> Result<Vec<Statement>, ParseError> {
    let lines: Vec<String> = input.lines().map(|l| l.to_string()).collect();
    parse_tokens_with_lines(lex(input), Some(&lines))
}

/// Parse G-code from a token iterator.
pub fn parse_tokens<I>(tokens: I) -> Result<Vec<Statement>, ParseError>
where
    I: IntoIterator<Item = Result<Token, LexError>>,
{
    parse_tokens_with_lines(tokens, None)
}

fn parse_tokens_with_lines<I>(
    tokens: I,
    lines: Option<&[String]>,
) -> Result<Vec<Statement>, ParseError>
where
    I: IntoIterator<Item = Result<Token, LexError>>,
{
    let mut out = Vec::new();
    let mut words = Vec::new();
    let mut comment: Option<String> = None;
    let mut checksum: Option<u8> = None;
    let mut current_line = 1usize;

    let flush = |target_line: usize,
                 words: &mut Vec<Word>,
                 comment: &mut Option<String>,
                 checksum: &mut Option<u8>,
                 out: &mut Vec<Statement>| {
        if words.is_empty() && comment.is_none() && checksum.is_none() {
            return;
        }
        let raw = lines
            .and_then(|ls| ls.get(target_line.saturating_sub(1)))
            .map(|s| s.trim_end().to_string())
            .unwrap_or_default();
        out.push(Statement {
            line: target_line,
            raw,
            words: std::mem::take(words),
            comment: comment.take(),
            checksum: checksum.take(),
        });
    };

    for token in tokens.into_iter() {
        let token = token?;
        current_line = token.line;
        match token.kind {
            TokenKind::Newline => {
                flush(
                    current_line,
                    &mut words,
                    &mut comment,
                    &mut checksum,
                    &mut out,
                );
            }
            TokenKind::Comment(text) => {
                if comment.is_some() {
                    return Err(ParseError::MultipleComments { line: current_line });
                }
                comment = Some(text);
            }
            TokenKind::Checksum(value) => {
                if checksum.is_some() {
                    return Err(ParseError::MultipleChecksums { line: current_line });
                }
                checksum = Some(value);
            }
            TokenKind::Word { letter, value } => {
                words.push(Word {
                    letter,
                    name: None,
                    value,
                });
            }
            TokenKind::Param { name, value } => {
                words.push(Word {
                    letter: None,
                    name: Some(name),
                    value,
                });
            }
        }
    }

    flush(
        current_line,
        &mut words,
        &mut comment,
        &mut checksum,
        &mut out,
    );
    Ok(out)
}
