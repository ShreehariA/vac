//! Hand-rolled lexer + recursive-descent parser for the v0.1 `.vac`
//! grammar. No external grammar library (pest/nom) on purpose — we
//! want zero dependencies for the format crate, and the v0.1 grammar
//! is tiny enough to read top-to-bottom in one file.
//!
//! Grammar (informal, EBNF-ish):
//!
//! ```text
//! document      = canvas_decl scene*
//! canvas_decl   = "canvas" Dimension "@" Fps
//! scene         = "scene" Ident "{" scene_stmt* "}"
//! scene_stmt    = duration_stmt | let_stmt | assign_stmt | animate_stmt
//! duration_stmt = "duration" Time
//! let_stmt      = "let" Ident "=" shape_expr
//! shape_expr    = ("rect" | "ellipse") "(" Number ("," Number){3} ")"
//! assign_stmt   = Ident "." Ident "=" value
//! value         = HexColor | "none"
//! animate_stmt  = "animate" Ident "{" keyframe* easing_decl? "}"
//! keyframe      = Time "->" transform ("," transform)*
//! transform     = "position" "(" Number "," Number ")"
//!               | "scale"    "(" Number ")"
//!               | "opacity"  "(" Number ")"
//! easing_decl   = "easing" ":" Ident
//! ```

use crate::ast::*;

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("{line}:{col}: {msg}")]
    Syntax {
        line: usize,
        col: usize,
        msg: String,
    },
}

impl ParseError {
    fn syntax(span: Span, msg: impl Into<String>) -> Self {
        ParseError::Syntax {
            line: span.line,
            col: span.col,
            msg: msg.into(),
        }
    }
}

// --------------------------------------------------------------------
// Tokens
// --------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Number(f32),
    /// `1500ms` or `3s` — normalised to milliseconds.
    TimeMs(u32),
    /// `30fps`.
    Fps(u32),
    /// `800x600`.
    Dimension(u32, u32),
    HexColor(Color),

    LBrace,
    RBrace,
    LParen,
    RParen,
    Comma,
    Equals,
    Dot,
    At,
    Colon,
    Arrow,

    Eof,
}

#[derive(Debug, Clone, Copy)]
struct Span {
    line: usize,
    col: usize,
}

#[derive(Debug, Clone)]
struct Spanned {
    tok: Token,
    span: Span,
}

// --------------------------------------------------------------------
// Lexer
// --------------------------------------------------------------------

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn span(&self) -> Span {
        Span {
            line: self.line,
            col: self.col,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' => {
                    self.bump();
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Spanned, ParseError> {
        self.skip_trivia();
        let span = self.span();
        let Some(c) = self.peek() else {
            return Ok(Spanned {
                tok: Token::Eof,
                span,
            });
        };

        // Single-char punctuation
        let single = match c {
            b'{' => Some(Token::LBrace),
            b'}' => Some(Token::RBrace),
            b'(' => Some(Token::LParen),
            b')' => Some(Token::RParen),
            b',' => Some(Token::Comma),
            b'=' => Some(Token::Equals),
            b'.' => Some(Token::Dot),
            b'@' => Some(Token::At),
            b':' => Some(Token::Colon),
            _ => None,
        };
        if let Some(tok) = single {
            self.bump();
            return Ok(Spanned { tok, span });
        }

        // Arrow `->`
        if c == b'-' {
            if self.peek_at(1) == Some(b'>') {
                self.bump();
                self.bump();
                return Ok(Spanned {
                    tok: Token::Arrow,
                    span,
                });
            }
            return Err(ParseError::syntax(
                span,
                "unexpected '-' (only '->' is allowed)",
            ));
        }

        // Hex color `#rrggbb` or `#rrggbbaa`
        if c == b'#' {
            return self.read_hex_color(span);
        }

        // Number / time / fps / dimension
        if c.is_ascii_digit() {
            return self.read_number_like(span);
        }

        // Identifier (and identifier-like keywords)
        if c.is_ascii_alphabetic() || c == b'_' {
            return Ok(self.read_ident(span));
        }

        Err(ParseError::syntax(
            span,
            format!("unexpected character {:?}", c as char),
        ))
    }

    fn read_hex_color(&mut self, span: Span) -> Result<Spanned, ParseError> {
        self.bump(); // consume '#'
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_hexdigit() {
                self.bump();
            } else {
                break;
            }
        }
        let hex = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        let color = match hex.len() {
            6 => {
                let v = u32::from_str_radix(hex, 16)
                    .map_err(|_| ParseError::syntax(span, "invalid hex color"))?;
                Color::rgb(((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)
            }
            8 => {
                let v = u32::from_str_radix(hex, 16)
                    .map_err(|_| ParseError::syntax(span, "invalid hex color"))?;
                Color::rgba(
                    ((v >> 24) & 0xFF) as u8,
                    ((v >> 16) & 0xFF) as u8,
                    ((v >> 8) & 0xFF) as u8,
                    (v & 0xFF) as u8,
                )
            }
            n => {
                return Err(ParseError::syntax(
                    span,
                    format!("hex color must have 6 or 8 digits, got {n}"),
                ))
            }
        };
        Ok(Spanned {
            tok: Token::HexColor(color),
            span,
        })
    }

    /// Read a leading number and look for unit suffixes:
    /// - `<n>ms`, `<n>s`           → TimeMs
    /// - `<n>fps`                  → Fps
    /// - `<n>x<m>`                 → Dimension
    /// - bare                      → Number (f32)
    fn read_number_like(&mut self, span: Span) -> Result<Spanned, ParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == b'.' {
                self.bump();
            } else {
                break;
            }
        }
        let lit = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        let value: f32 = lit
            .parse()
            .map_err(|_| ParseError::syntax(span, format!("invalid number {lit:?}")))?;

        // `x<digits>` → dimension
        if self.peek() == Some(b'x')
            && self.peek_at(1).map(|c| c.is_ascii_digit()).unwrap_or(false)
        {
            self.bump(); // consume 'x'
            let start2 = self.pos;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.bump();
                } else {
                    break;
                }
            }
            let h_lit = std::str::from_utf8(&self.src[start2..self.pos]).unwrap_or("");
            let h: u32 = h_lit
                .parse()
                .map_err(|_| ParseError::syntax(span, "invalid dimension height"))?;
            let w = value as u32;
            return Ok(Spanned {
                tok: Token::Dimension(w, h),
                span,
            });
        }

        // Letter suffix?  `ms` / `s` / `fps`
        let unit_start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                self.bump();
            } else {
                break;
            }
        }
        let unit = std::str::from_utf8(&self.src[unit_start..self.pos]).unwrap_or("");

        let tok = match unit {
            "" => Token::Number(value),
            "ms" => Token::TimeMs(value.round() as u32),
            "s" => Token::TimeMs((value * 1000.0).round() as u32),
            "fps" => Token::Fps(value.round() as u32),
            other => {
                return Err(ParseError::syntax(
                    span,
                    format!("unknown numeric unit {other:?} (expected ms/s/fps)"),
                ))
            }
        };
        Ok(Spanned { tok, span })
    }

    fn read_ident(&mut self, span: Span) -> Spanned {
        let start = self.pos;
        // First char already validated as alpha/_; consume it then trailing
        // `[a-zA-Z0-9_-]*` (hyphens allowed for `ease-in-out` etc.).
        self.bump();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                self.bump();
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("")
            .to_string();
        Spanned {
            tok: Token::Ident(s),
            span,
        }
    }
}

// --------------------------------------------------------------------
// Parser
// --------------------------------------------------------------------

struct Parser {
    tokens: Vec<Spanned>,
    cursor: usize,
}

impl Parser {
    fn new(tokens: Vec<Spanned>) -> Self {
        Parser { tokens, cursor: 0 }
    }

    fn peek(&self) -> &Spanned {
        &self.tokens[self.cursor]
    }

    fn bump(&mut self) -> Spanned {
        let t = self.tokens[self.cursor].clone();
        if !matches!(t.tok, Token::Eof) {
            self.cursor += 1;
        }
        t
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<Span, ParseError> {
        let s = self.peek().clone();
        if let Token::Ident(ref name) = s.tok {
            if name == kw {
                self.bump();
                return Ok(s.span);
            }
        }
        Err(ParseError::syntax(
            s.span,
            format!("expected keyword `{kw}`, got {:?}", s.tok),
        ))
    }

    fn expect_ident(&mut self) -> Result<(String, Span), ParseError> {
        let s = self.peek().clone();
        if let Token::Ident(name) = s.tok {
            self.bump();
            Ok((name, s.span))
        } else {
            Err(ParseError::syntax(
                s.span,
                format!("expected identifier, got {:?}", s.tok),
            ))
        }
    }

    fn expect(&mut self, target: &Token) -> Result<Span, ParseError> {
        let s = self.peek().clone();
        if std::mem::discriminant(&s.tok) == std::mem::discriminant(target) {
            self.bump();
            Ok(s.span)
        } else {
            Err(ParseError::syntax(
                s.span,
                format!("expected {target:?}, got {:?}", s.tok),
            ))
        }
    }

    fn expect_number(&mut self) -> Result<f32, ParseError> {
        let s = self.peek().clone();
        match s.tok {
            Token::Number(n) => {
                self.bump();
                Ok(n)
            }
            // Accept integer-typed dimensions/times in number positions only
            // for explicit numeric lists: caller decides to use a strict
            // expect_number, so nothing else is accepted here.
            other => Err(ParseError::syntax(
                s.span,
                format!("expected number, got {other:?}"),
            )),
        }
    }

    fn expect_time_ms(&mut self) -> Result<u32, ParseError> {
        let s = self.peek().clone();
        match s.tok {
            Token::TimeMs(t) => {
                self.bump();
                Ok(t)
            }
            other => Err(ParseError::syntax(
                s.span,
                format!("expected time (e.g. 500ms or 2s), got {other:?}"),
            )),
        }
    }

    // ---------- top-level ----------

    fn parse_document(&mut self) -> Result<Document, ParseError> {
        let canvas = self.parse_canvas_decl()?;
        let mut scenes = Vec::new();
        while !matches!(self.peek().tok, Token::Eof) {
            scenes.push(self.parse_scene()?);
        }
        Ok(Document { canvas, scenes })
    }

    fn parse_canvas_decl(&mut self) -> Result<Canvas, ParseError> {
        self.expect_keyword("canvas")?;
        let s = self.peek().clone();
        let (width, height) = match s.tok {
            Token::Dimension(w, h) => {
                self.bump();
                (w, h)
            }
            other => {
                return Err(ParseError::syntax(
                    s.span,
                    format!("expected canvas size like `800x600`, got {other:?}"),
                ))
            }
        };
        self.expect(&Token::At)?;
        let s = self.peek().clone();
        let fps = match s.tok {
            Token::Fps(f) => {
                self.bump();
                f
            }
            other => {
                return Err(ParseError::syntax(
                    s.span,
                    format!("expected fps like `30fps`, got {other:?}"),
                ))
            }
        };
        Ok(Canvas { width, height, fps })
    }

    fn parse_scene(&mut self) -> Result<Scene, ParseError> {
        self.expect_keyword("scene")?;
        let (name, _) = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut duration_ms: Option<u32> = None;
        let mut statements = Vec::new();

        loop {
            let s = self.peek().clone();
            match &s.tok {
                Token::RBrace => {
                    self.bump();
                    break;
                }
                Token::Ident(kw) if kw == "duration" => {
                    self.bump();
                    let t = self.expect_time_ms()?;
                    if duration_ms.is_some() {
                        return Err(ParseError::syntax(
                            s.span,
                            "duration declared twice in scene",
                        ));
                    }
                    duration_ms = Some(t);
                }
                Token::Ident(kw) if kw == "let" => {
                    self.bump();
                    statements.push(self.parse_let_after_keyword()?);
                }
                Token::Ident(kw) if kw == "animate" => {
                    self.bump();
                    statements.push(Statement::Animate(self.parse_animate_after_keyword()?));
                }
                Token::Ident(_) => {
                    statements.push(self.parse_assign()?);
                }
                other => {
                    return Err(ParseError::syntax(
                        s.span,
                        format!("unexpected token in scene: {other:?}"),
                    ));
                }
            }
        }

        Ok(Scene {
            name,
            duration_ms: duration_ms.unwrap_or(0),
            statements,
        })
    }

    fn parse_let_after_keyword(&mut self) -> Result<Statement, ParseError> {
        let (name, _) = self.expect_ident()?;
        self.expect(&Token::Equals)?;
        let shape = self.parse_shape_expr()?;
        Ok(Statement::Let { name, shape })
    }

    fn parse_shape_expr(&mut self) -> Result<Shape, ParseError> {
        let (kind, span) = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let nums = self.parse_number_list()?;
        self.expect(&Token::RParen)?;
        match kind.as_str() {
            "rect" => {
                if nums.len() != 4 {
                    return Err(ParseError::syntax(
                        span,
                        format!("rect() takes 4 args (x, y, w, h), got {}", nums.len()),
                    ));
                }
                Ok(Shape::Rect {
                    x: nums[0],
                    y: nums[1],
                    w: nums[2],
                    h: nums[3],
                })
            }
            "ellipse" => {
                if nums.len() != 4 {
                    return Err(ParseError::syntax(
                        span,
                        format!(
                            "ellipse() takes 4 args (cx, cy, rx, ry), got {}",
                            nums.len()
                        ),
                    ));
                }
                Ok(Shape::Ellipse {
                    cx: nums[0],
                    cy: nums[1],
                    rx: nums[2],
                    ry: nums[3],
                })
            }
            other => Err(ParseError::syntax(
                span,
                format!("unknown shape primitive `{other}` (expected rect or ellipse)"),
            )),
        }
    }

    fn parse_number_list(&mut self) -> Result<Vec<f32>, ParseError> {
        let mut out = Vec::new();
        if matches!(self.peek().tok, Token::RParen) {
            return Ok(out);
        }
        out.push(self.expect_number()?);
        while matches!(self.peek().tok, Token::Comma) {
            self.bump();
            out.push(self.expect_number()?);
        }
        Ok(out)
    }

    fn parse_assign(&mut self) -> Result<Statement, ParseError> {
        let (target, span) = self.expect_ident()?;
        self.expect(&Token::Dot)?;
        let (prop_name, _) = self.expect_ident()?;
        self.expect(&Token::Equals)?;
        let property = Property::from_str(&prop_name).ok_or_else(|| {
            ParseError::syntax(
                span,
                format!("unknown property `{prop_name}` (expected fill or stroke)"),
            )
        })?;
        let value = self.parse_value()?;
        Ok(Statement::Assign {
            target,
            property,
            value,
        })
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        let s = self.peek().clone();
        match s.tok {
            Token::HexColor(c) => {
                self.bump();
                Ok(Value::Color(c))
            }
            Token::Ident(ref name) if name == "none" => {
                self.bump();
                Ok(Value::None)
            }
            other => Err(ParseError::syntax(
                s.span,
                format!("expected color or `none`, got {other:?}"),
            )),
        }
    }

    fn parse_animate_after_keyword(&mut self) -> Result<Animation, ParseError> {
        let (target, _) = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut keyframes = Vec::new();
        let mut easing = Easing::Linear;

        loop {
            let s = self.peek().clone();
            match &s.tok {
                Token::RBrace => {
                    self.bump();
                    break;
                }
                Token::TimeMs(_) => {
                    let kf = self.parse_keyframe()?;
                    keyframes.push(kf);
                }
                Token::Ident(kw) if kw == "easing" => {
                    self.bump();
                    self.expect(&Token::Colon)?;
                    let (name, ident_span) = self.expect_ident()?;
                    easing = Easing::from_str(&name).ok_or_else(|| {
                        ParseError::syntax(ident_span, format!("unknown easing `{name}`"))
                    })?;
                }
                other => {
                    return Err(ParseError::syntax(
                        s.span,
                        format!("unexpected token in animate body: {other:?}"),
                    ));
                }
            }
        }

        Ok(Animation {
            target,
            keyframes,
            easing,
        })
    }

    fn parse_keyframe(&mut self) -> Result<Keyframe, ParseError> {
        let time_ms = self.expect_time_ms()?;
        self.expect(&Token::Arrow)?;
        let mut transforms = Vec::new();
        transforms.push(self.parse_transform()?);
        while matches!(self.peek().tok, Token::Comma) {
            self.bump();
            transforms.push(self.parse_transform()?);
        }
        Ok(Keyframe {
            time_ms,
            transforms,
        })
    }

    fn parse_transform(&mut self) -> Result<Transform, ParseError> {
        let (kind, span) = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let nums = self.parse_number_list()?;
        self.expect(&Token::RParen)?;
        match kind.as_str() {
            "position" => {
                if nums.len() != 2 {
                    return Err(ParseError::syntax(
                        span,
                        format!("position() takes (x, y), got {} args", nums.len()),
                    ));
                }
                Ok(Transform::Position(nums[0], nums[1]))
            }
            "scale" => {
                if nums.len() != 1 {
                    return Err(ParseError::syntax(
                        span,
                        format!("scale() takes 1 arg, got {}", nums.len()),
                    ));
                }
                Ok(Transform::Scale(nums[0]))
            }
            "opacity" => {
                if nums.len() != 1 {
                    return Err(ParseError::syntax(
                        span,
                        format!("opacity() takes 1 arg, got {}", nums.len()),
                    ));
                }
                Ok(Transform::Opacity(nums[0]))
            }
            other => Err(ParseError::syntax(
                span,
                format!("unknown transform `{other}` (expected position/scale/opacity)"),
            )),
        }
    }
}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Parse `.vac` source text into a [`Document`].
pub fn parse(src: &str) -> Result<Document, ParseError> {
    let mut lexer = Lexer::new(src);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token()?;
        let is_eof = matches!(t.tok, Token::Eof);
        tokens.push(t);
        if is_eof {
            break;
        }
    }
    let mut parser = Parser::new(tokens);
    parser.parse_document()
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer;

    const SAMPLE: &str = r#"
canvas 800x600 @30fps

scene main {
  duration 3s

  let bg = rect(0, 0, 800, 600)
  bg.fill = #1a1a2e

  let ball = ellipse(100, 300, 30, 30)
  ball.fill = #e94560
  ball.stroke = none

  animate ball {
    0ms -> position(100, 300)
    1500ms -> position(700, 300)
    3s -> position(100, 300)
    easing: linear
  }
}
"#;

    #[test]
    fn parses_sample() {
        let doc = parse(SAMPLE).expect("parse");
        assert_eq!(doc.canvas.width, 800);
        assert_eq!(doc.canvas.height, 600);
        assert_eq!(doc.canvas.fps, 30);
        assert_eq!(doc.scenes.len(), 1);
        assert_eq!(doc.scenes[0].name, "main");
        assert_eq!(doc.scenes[0].duration_ms, 3000);
    }

    #[test]
    fn round_trip_write_then_parse() {
        let original = parse(SAMPLE).unwrap();
        let text = writer::write(&original);
        let reparsed = parse(&text).expect("re-parse writer output");
        assert_eq!(original, reparsed, "round-trip lost data");
    }

    #[test]
    fn rejects_unknown_easing() {
        let bad = r#"
canvas 100x100 @30fps
scene s {
  duration 1s
  let b = ellipse(50, 50, 10, 10)
  animate b {
    0ms -> position(0, 0)
    1s  -> position(100, 100)
    easing: bouncy
  }
}
"#;
        assert!(parse(bad).is_err());
    }

    #[test]
    fn comments_are_skipped() {
        let s = r#"
// header comment
canvas 100x100 @30fps   // trailing comment
scene s {
  // body
  duration 1s
  let b = ellipse(50, 50, 10, 10) // shape
}
"#;
        let doc = parse(s).expect("parse with comments");
        assert_eq!(doc.scenes[0].statements.len(), 1);
    }
}
