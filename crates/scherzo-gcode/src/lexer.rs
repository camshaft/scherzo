use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum TokenKind {
    Word {
        letter: Option<char>,
        value: Option<Value>,
    },
    Param {
        name: String,
        value: Option<Value>,
    },
    Comment(String),
    Checksum(u8),
    Newline,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum Value {
    Number(Number),
    Text(String),
    List(Vec<Value>),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum Number {
    Int(i64),
    Float(f64),
}

#[derive(Debug, Error)]
pub enum LexError {
    #[error("unexpected character '{ch}' at line {line}, column {column}")]
    UnexpectedChar {
        line: usize,
        column: usize,
        ch: char,
    },

    #[error("invalid number '{raw}' at line {line}, column {column}")]
    InvalidNumber {
        line: usize,
        column: usize,
        raw: String,
        #[source]
        source: std::num::ParseFloatError,
    },

    #[error("invalid checksum '{raw}' at line {line}, column {column}")]
    InvalidChecksum {
        line: usize,
        column: usize,
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },

    #[error("unterminated parenthesized comment starting at line {line}, column {column}")]
    UnterminatedComment { line: usize, column: usize },

    #[error("unterminated quoted string starting at line {line}, column {column}")]
    UnterminatedString { line: usize, column: usize },
}

pub fn lex(input: &str) -> Lexer<'_> {
    Lexer::new(input)
}

pub struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
            line: 1,
            column: 1,
        }
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn pos(&self) -> (usize, usize) {
        (self.line, self.column)
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Token, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ch) = self.peek() {
            let (line, column) = self.pos();

            if ch.is_ascii_whitespace() {
                self.bump();
                if ch == '\n' {
                    return Some(Ok(Token {
                        kind: TokenKind::Newline,
                        line,
                        column,
                    }));
                }
                continue;
            }

            if ch == ';' || ch == '#' {
                // Inline comment until end of line
                self.bump();
                let mut text = String::new();
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    text.push(c);
                    self.bump();
                }
                return Some(Ok(Token {
                    kind: TokenKind::Comment(text.trim_start().to_string()),
                    line,
                    column,
                }));
            }

            if ch == '(' {
                self.bump();
                let mut text = String::new();
                while let Some(c) = self.peek() {
                    if c == ')' {
                        self.bump();
                        return Some(Ok(Token {
                            kind: TokenKind::Comment(text),
                            line,
                            column,
                        }));
                    }
                    text.push(c);
                    self.bump();
                }
                return Some(Err(LexError::UnterminatedComment { line, column }));
            }

            if ch == '*' {
                self.bump();
                let start_col = column + 1;
                let mut raw = String::new();
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    raw.push(c);
                    self.bump();
                }
                let raw_trimmed = raw.trim();
                if raw_trimmed.is_empty() {
                    return Some(Err(LexError::InvalidChecksum {
                        line,
                        column: start_col,
                        raw,
                        source: "".parse::<u8>().unwrap_err(),
                    }));
                }
                match raw_trimmed.parse::<u32>() {
                    Ok(v) if v <= u8::MAX as u32 => {
                        return Some(Ok(Token {
                            kind: TokenKind::Checksum(v as u8),
                            line,
                            column,
                        }));
                    }
                    Ok(v) => {
                        return Some(Err(LexError::InvalidChecksum {
                            line,
                            column: start_col,
                            raw: v.to_string(),
                            source: "".parse::<u8>().unwrap_err(),
                        }));
                    }
                    Err(source) => {
                        return Some(Err(LexError::InvalidChecksum {
                            line,
                            column: start_col,
                            raw: raw_trimmed.to_string(),
                            source,
                        }));
                    }
                }
            }

            if ch.is_ascii_alphabetic() {
                self.bump();
                let letter = ch;

                if let Some(next) = self.peek() {
                    if is_number_start(next) {
                        let start_col = self.column;
                        let parsed = match parse_number(self) {
                            Ok(res) => res,
                            Err(err) => return Some(Err(err.with_position(line, start_col))),
                        };
                        return Some(Ok(Token {
                            kind: TokenKind::Word {
                                letter: Some(letter),
                                value: Some(Value::Number(parsed)),
                            },
                            line,
                            column,
                        }));
                    }

                    if next == '"' {
                        let start_col = self.column + 1; // quote is at current column
                        self.bump(); // consume opening quote
                        match parse_quoted_string(self) {
                            Ok(text) => {
                                return Some(Ok(Token {
                                    kind: TokenKind::Word {
                                        letter: Some(letter),
                                        value: Some(Value::Text(text)),
                                    },
                                    line,
                                    column,
                                }));
                            }
                            Err(err) => return Some(Err(err.with_position(line, start_col))),
                        }
                    }
                }

                // Identifier-style token: consume the rest of the run
                let mut raw = String::new();
                raw.push(letter);
                while let Some(c) = self.peek() {
                    if is_value_terminator(c) {
                        break;
                    }
                    raw.push(c);
                    self.bump();
                }

                return Some(Ok(token_from_raw(line, column, raw)));
            }

            if ch == '"' {
                // Bare string literal used by commands like M117
                let start_col = column;
                self.bump(); // consume opening quote
                match parse_quoted_string(self) {
                    Ok(text) => {
                        return Some(Ok(Token {
                            kind: TokenKind::Word {
                                letter: None,
                                value: Some(Value::Text(text)),
                            },
                            line,
                            column: start_col,
                        }));
                    }
                    Err(err) => return Some(Err(err.with_position(line, start_col + 1))),
                }
            }

            // Fallback: treat any other non-whitespace, non-comment-leading char as a bare text token
            let mut raw = String::new();
            while let Some(c) = self.peek() {
                if is_value_terminator(c) {
                    break;
                }
                raw.push(c);
                self.bump();
            }
            if !raw.is_empty() {
                return Some(Ok(token_from_raw(line, column, raw)));
            }

            return Some(Err(LexError::UnexpectedChar { line, column, ch }));
        }

        None
    }
}

fn is_number_start(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.')
}

fn is_value_terminator(ch: char) -> bool {
    ch.is_ascii_whitespace() || matches!(ch, ';' | '(' | '*' | '#')
}

fn parse_number(lexer: &mut Lexer<'_>) -> Result<Number, PositionedErrorKind> {
    let mut raw = String::new();
    if matches!(lexer.peek(), Some(ch) if matches!(ch, '+' | '-')) {
        raw.push(lexer.peek().unwrap());
        lexer.bump();
    }

    let mut has_digit = false;
    while let Some(ch) = lexer.peek() {
        if ch.is_ascii_digit() {
            has_digit = true;
            raw.push(ch);
            lexer.bump();
        } else {
            break;
        }
    }

    if let Some('.') = lexer.peek() {
        raw.push('.');
        lexer.bump();
        while let Some(ch) = lexer.peek() {
            if ch.is_ascii_digit() {
                has_digit = true;
                raw.push(ch);
                lexer.bump();
            } else {
                break;
            }
        }
    }

    if matches!(lexer.peek(), Some(ch) if matches!(ch, 'e' | 'E')) {
        raw.push(lexer.peek().unwrap());
        lexer.bump();

        if matches!(lexer.peek(), Some(sign) if matches!(sign, '+' | '-')) {
            raw.push(lexer.peek().unwrap());
            lexer.bump();
        }

        let mut exp_digit = false;
        while let Some(d) = lexer.peek() {
            if d.is_ascii_digit() {
                exp_digit = true;
                raw.push(d);
                lexer.bump();
            } else {
                break;
            }
        }
        has_digit |= exp_digit;
    }

    if !has_digit {
        return Err(PositionedErrorKind::InvalidNumber { raw: raw.clone() });
    }

    let is_floaty = raw.contains('.') || raw.contains('e') || raw.contains('E');

    if let (false, Ok(int)) = (is_floaty, raw.parse::<i64>()) {
        return Ok(Number::Int(int));
    }

    let parsed =
        raw.parse::<f64>()
            .map_err(|source| PositionedErrorKind::InvalidNumberWithSource {
                raw: raw.clone(),
                source,
            })?;

    Ok(Number::Float(parsed))
}

fn parse_quoted_string(lexer: &mut Lexer<'_>) -> Result<String, PositionedErrorKind> {
    let mut text = String::new();
    while let Some(ch) = lexer.peek() {
        if ch == '"' {
            lexer.bump();
            return Ok(text);
        }
        if ch == '\n' {
            return Err(PositionedErrorKind::UnterminatedString);
        }
        text.push(ch);
        lexer.bump();
    }
    Err(PositionedErrorKind::UnterminatedString)
}

fn token_from_raw(line: usize, column: usize, raw: String) -> Token {
    if let Some((name, value_str)) = raw.split_once('=') {
        let value = if value_str.is_empty() {
            None
        } else {
            match parse_value_string(value_str) {
                Ok(v) => Some(v),
                Err(_) => Some(Value::Text(value_str.to_string())),
            }
        };
        Token {
            kind: TokenKind::Param {
                name: name.to_string(),
                value,
            },
            line,
            column,
        }
    } else {
        Token {
            kind: TokenKind::Word {
                letter: None,
                value: Some(Value::Text(raw)),
            },
            line,
            column,
        }
    }
}

fn parse_value_string(raw: &str) -> Result<Value, PositionedErrorKind> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Value::Text(String::new()));
    }

    if let Some(parsed) = parse_scalar_value(raw) {
        return Ok(parsed);
    }

    // Attempt list parsing (comma-separated, respecting simple quotes)
    if raw.contains(',') {
        let mut items = Vec::new();
        let mut buf = String::new();
        let mut current_quote: Option<char> = None;
        for ch in raw.chars() {
            match ch {
                '\'' | '"' => {
                    if current_quote == Some(ch) {
                        current_quote = None;
                    } else if current_quote.is_none() {
                        current_quote = Some(ch);
                    }
                    buf.push(ch);
                }
                ',' if current_quote.is_none() => {
                    let trimmed = buf.trim();
                    if !trimmed.is_empty() {
                        if let Some(val) = parse_scalar_value(trimmed) {
                            items.push(val);
                        } else {
                            items.push(Value::Text(trimmed.to_string()));
                        }
                    } else {
                        items.push(Value::Text(String::new()));
                    }
                    buf.clear();
                }
                _ => buf.push(ch),
            }
        }
        if !buf.is_empty() {
            let trimmed = buf.trim();
            if !trimmed.is_empty() {
                if let Some(val) = parse_scalar_value(trimmed) {
                    items.push(val);
                } else {
                    items.push(Value::Text(trimmed.to_string()));
                }
            } else {
                items.push(Value::Text(String::new()));
            }
        }

        if !items.is_empty() {
            return Ok(Value::List(items));
        }
    }

    Ok(Value::Text(raw.to_string()))
}

fn parse_scalar_value(raw: &str) -> Option<Value> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(Value::Text(String::new()));
    }

    // Quoted string (single or double)
    if (raw.starts_with('"') && raw.ends_with('"'))
        || (raw.starts_with('\'') && raw.ends_with('\''))
    {
        let inner = &raw[1..raw.len() - 1];
        return Some(Value::Text(inner.to_string()));
    }

    if let Ok(int) = raw.parse::<i64>() {
        return Some(Value::Number(Number::Int(int)));
    }
    if let Ok(flt) = raw.parse::<f64>() {
        return Some(Value::Number(Number::Float(flt)));
    }
    None
}

#[derive(Debug)]
enum PositionedErrorKind {
    InvalidNumber {
        raw: String,
    },
    InvalidNumberWithSource {
        raw: String,
        source: std::num::ParseFloatError,
    },
    UnterminatedString,
}

impl PositionedErrorKind {
    fn with_position(self, line: usize, column: usize) -> LexError {
        match self {
            PositionedErrorKind::InvalidNumber { raw } => LexError::InvalidNumber {
                line,
                column,
                raw,
                source: "".parse::<f64>().unwrap_err(),
            },
            PositionedErrorKind::InvalidNumberWithSource { raw, source } => {
                LexError::InvalidNumber {
                    line,
                    column,
                    raw,
                    source,
                }
            }
            PositionedErrorKind::UnterminatedString => {
                LexError::UnterminatedString { line, column }
            }
        }
    }
}
