//! # JSON Lexer
//!
//! Parses an input byte sequence from a JSON document into a sequence of
//! tokens, along with information with the amount of bytes processed from the
//! input.
use crate::tokenizer::JToken;

/// A lexer that can be used to parse an input slice of bytes from a JSON
/// document into tokens.
struct Lexer<'a> {
    /// The input sequence of bytes to tokenize
    input: &'a [u8],
    /// Current position (current byte)
    position: usize,
    /// Current reading position (after current byte)
    read_position: usize,
    /// Current byte under examination
    byte: u8,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a [u8]) -> Self {
        let mut lexer = Self {
            input,
            position: 0,
            read_position: 0,
            byte: 0,
        };
        // put the lexer in an initial working state
        lexer.read_byte();
        lexer
    }

    /// Reads and consumes the next byte in the input sequence.
    fn read_byte(&mut self) {
        if self.read_position >= self.input.len() {
            self.byte = 0 // EOF
        } else {
            self.byte = self.input[self.read_position];
        }
        // Advance the positions
        self.position = self.read_position;
        self.read_position += 1;
    }

    /// Consume whitespace byte(s) starting from the current position.
    fn skip_whitespace(&mut self) {
        while matches!(self.byte, b' ' | b'\t' | b'\n' | b'\r') {
            self.read_byte();
        }
    }

    /// Returns the next token in the input sequence from the current position.
    fn next_token(&mut self) -> JToken {
        self.skip_whitespace();

        match self.byte {
            0 => JToken::Eof, // `read_byte` marked EOF
            b'{' => {
                self.read_byte();
                JToken::LCurly
            }
            b'}' => {
                self.read_byte();
                JToken::RCurly
            }
            b'[' => {
                self.read_byte();
                JToken::LSquare
            }
            b']' => {
                self.read_byte();
                JToken::RSquare
            }
            b':' => {
                self.read_byte();
                JToken::Colon
            }
            b',' => {
                self.read_byte();
                JToken::Comma
            }
            b'"' => self.read_string(),
            b'-' | b'0'..=b'9' => self.read_number(),
            c if c.is_ascii_alphabetic() => self.read_literal(),
            _ => {
                self.read_byte();
                JToken::Illegal
            }
        }
    }

    /// Reads an alphabetic literal (`true`/`false`/`null`) and returns the
    /// corresponding token.
    fn read_literal(&mut self) -> JToken {
        let start_pos = self.position;
        while self.byte.is_ascii_alphabetic() {
            self.read_byte();
        }
        let slice = &self.input[start_pos..self.position];
        match slice {
            b"true" => JToken::Bool(true),
            b"false" => JToken::Bool(false),
            b"null" => JToken::Null,
            _ => JToken::Illegal,
        }
    }

    /// Reads a string value and returns the corresponding token.
    fn read_string(&mut self) -> JToken {
        // Skip opening quote
        let start_pos = self.position + 1;
        self.read_byte();
        while !matches!(self.byte, b'"') && self.byte != 0 {
            // escape sequence with backslash literal
            if self.byte == b'\\' {
                // skip the escaped character to avoid premature termination
                // with `\"`
                self.read_byte();
            }
            self.read_byte();
        }

        if self.byte == 0 {
            // string not terminated, invalid
            return JToken::Illegal;
        }

        let end_pos = self.position - 1;
        self.read_byte();

        JToken::JString(start_pos, end_pos)
    }

    /// Reads a JSON number (int, frac, exp) and returns a JNumber token.
    fn read_number(&mut self) -> JToken {
        let start_pos = self.position;

        // optional leading '-'
        if self.byte == b'-' {
            self.read_byte();
        }

        // integer part
        while self.byte.is_ascii_digit() {
            self.read_byte();
        }

        // fractional part
        if self.byte == b'.' {
            self.read_byte();
            while self.byte.is_ascii_digit() {
                self.read_byte();
            }
        }

        // exponent part
        if matches!(self.byte, b'e' | b'E') {
            self.read_byte();
            if matches!(self.byte, b'+' | b'-') {
                self.read_byte();
            }
            while self.byte.is_ascii_digit() {
                self.read_byte();
            }
        }

        // self.position is now one past the last digit,
        // so the end index is position - 1
        JToken::JNumber(start_pos, self.position - 1)
    }

    /// Returns the amount of bytes of the input sequence have been read.
    fn bytes_read(&self) -> usize {
        if self.byte == 0 && self.position > 0 {
            self.input.len()
        } else {
            std::cmp::min(self.position, self.input.len())
        }
    }
}

/// Tokenize a JSON document from bytes into tokens, returning both the token
/// sequence and the number of bytes of the input read.
pub fn tokenize(text: &[u8]) -> (Vec<JToken>, usize) {
    let mut lexer = Lexer::new(text);
    let mut tokens: Vec<JToken> = vec![];

    loop {
        let token = lexer.next_token();
        let is_eof = matches!(token, JToken::Eof);

        tokens.push(token);

        if is_eof {
            break;
        }
    }

    (tokens, lexer.bytes_read())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let input = "".as_bytes();
        let (tokens, bytes_read) = tokenize(input);

        assert_eq!(bytes_read, 0);
        assert_eq!(tokens.len(), 1); // Including EOF

        let expected: Vec<JToken> = vec![JToken::Eof];
        assert_eq!(expected, tokens)
    }

    #[test]
    fn test_literals() {
        let input = "null true false".as_bytes();
        let (tokens, _) = tokenize(input);
        assert_eq!(
            tokens,
            vec![
                JToken::Null,
                JToken::Bool(true),
                JToken::Bool(false),
                JToken::Eof
            ]
        )
    }

    #[test]
    fn test_number_variants() {
        let cases = [
            ("0", JToken::JNumber(0, 0)),
            ("-0", JToken::JNumber(0, 1)),
            ("123", JToken::JNumber(0, 2)),
            ("-123", JToken::JNumber(0, 3)),
            ("3.14", JToken::JNumber(0, 3)),
            ("0.001e-10", JToken::JNumber(0, 8)),
        ];
        for (s, expected) in &cases {
            let (toks, _) = tokenize(s.as_bytes());
            assert_eq!(&toks[..2], &[expected.clone(), JToken::Eof]);
        }
    }

    #[test]
    fn test_string_with_escape() {
        let input = br#""hello\nworld\"!""#;
        let (toks, _) = tokenize(input);
        // The content is from byte 1 to byte len-2
        let end = input.len() - 2;
        assert_eq!(toks, vec![JToken::JString(1, end), JToken::Eof,]);
    }

    #[test]
    fn test_escape_sequences() {
        // Test all standard JSON escape sequences, see `char > escape`:
        // https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/JSON#full_json_grammar
        let cases = [
            r#""Test \"quoted\" text""#,        // Double quote
            r#""Backslash: \\""#,               // Backslash
            r#""Forward slash: \/""#,           // Forward slash
            r#""Backspace: \b""#,               // Backspace
            r#""Form feed: \f""#,               // Form feed
            r#""Newline: \n""#,                 // Newline
            r#""Carriage return: \r""#,         // Carriage return
            r#""Tab: \t""#,                     // Tab
            r#""Unicode: \u0041\u0042\u0043""#, // Unicode escape
            r#""Mixed: \"\\\n\t\u0020""#,       // Mixed escapes
        ];

        for input in &cases {
            let (toks, _) = tokenize(input.as_bytes());
            assert_eq!(toks.len(), 2); // String token + EOF
            assert!(matches!(toks[0], JToken::JString(_, _)));
            assert!(matches!(toks[1], JToken::Eof));
        }
    }
}
