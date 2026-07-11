//! `.pac` text → AST.
//!
//! Hand-rolled recursive-descent parser; deliberately tiny so the
//! grammar surface area stays auditable.  Whitespace is insignificant
//! between tokens (newline-tolerant) so the user can re-flow a `.pac`
//! file in any editor without breaking it.
//!
//! Errors carry a line number to make hand-editing tractable.

use crate::ast::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("line {line}: {msg}")]
    Generic { line: u32, msg: String },
}

impl ParseError {
    fn at(line: u32, msg: impl Into<String>) -> Self {
        ParseError::Generic { line, msg: msg.into() }
    }
}

pub fn parse(src: &str) -> Result<Document, ParseError> {
    let mut p = Parser::new(src);
    p.parse_document()
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser { src: src.as_bytes(), pos: 0, line: 1 }
    }

    fn parse_document(&mut self) -> Result<Document, ParseError> {
        self.skip_trivia();
        let canvas = self.parse_canvas_header()?;
        let mut background: Option<Color> = None;
        let mut shapes: Vec<ShapeDecl> = Vec::new();

        loop {
            self.skip_trivia();
            if self.eof() {
                break;
            }
            if self.try_keyword("background") {
                self.skip_inline_ws();
                let c = self.parse_color()?;
                background = Some(c);
                continue;
            }
            if self.try_keyword("let") {
                let decl = self.parse_let_with_fill()?;
                shapes.push(decl);
                continue;
            }
            return Err(ParseError::at(
                self.line,
                format!(
                    "expected `background`, `let`, or EOF; got `{}`",
                    self.peek_word_for_error()
                ),
            ));
        }
        Ok(Document { canvas, background, shapes })
    }

    fn parse_canvas_header(&mut self) -> Result<Canvas, ParseError> {
        if !self.try_keyword("canvas") {
            return Err(ParseError::at(self.line, "expected `canvas` header"));
        }
        self.skip_inline_ws();
        let w = self.parse_uint()?;
        self.expect_char(b'x')?;
        let h = self.parse_uint()?;
        Ok(Canvas { width: w, height: h })
    }

    fn parse_let_with_fill(&mut self) -> Result<ShapeDecl, ParseError> {
        self.skip_inline_ws();
        let name = self.parse_identifier()?;
        self.skip_inline_ws();
        self.expect_char(b'=')?;
        self.skip_inline_ws();
        let shape = self.parse_shape()?;
        self.skip_trivia();

        // Require a fill assignment as the *next* statement.  We parse
        // it eagerly so the AST is always complete; the writer always
        // emits one so the round-trip is symmetric.
        let target = self.parse_identifier()?;
        if target != name {
            return Err(ParseError::at(
                self.line,
                format!("expected `{name}.fill = ...` after let; got assignment to `{target}`"),
            ));
        }
        self.expect_char(b'.')?;
        let prop = self.parse_identifier()?;
        if prop != "fill" {
            return Err(ParseError::at(self.line, format!("only `.fill` is supported (got `.{prop}`)")));
        }
        self.skip_inline_ws();
        self.expect_char(b'=')?;
        self.skip_inline_ws();
        let fill = self.parse_fill()?;
        Ok(ShapeDecl { name, shape, fill })
    }

    fn parse_fill(&mut self) -> Result<Fill, ParseError> {
        // `gradient(linear, #c0 at x,y, #c1 at x,y)` vs `#rrggbb`.
        if self.peek() == Some(b'#') {
            return Ok(Fill::Solid(self.parse_color()?));
        }
        // Otherwise expect `gradient(...)`.
        let kind = self.parse_identifier()?;
        if kind != "gradient" {
            return Err(ParseError::at(
                self.line,
                format!("expected `#rrggbb` or `gradient(...)`, got `{kind}`"),
            ));
        }
        self.expect_char(b'(')?;
        self.skip_inline_ws();
        let variant = self.parse_identifier()?;
        if variant != "linear" {
            return Err(ParseError::at(
                self.line,
                format!("only `gradient(linear, ...)` is supported in v0.2 (got `{variant}`)"),
            ));
        }
        self.skip_inline_ws();
        self.expect_char(b',')?;
        self.skip_inline_ws();
        let c0 = self.parse_color()?;
        self.skip_inline_ws();
        if !self.try_keyword("at") {
            return Err(ParseError::at(self.line, "expected `at <x>,<y>` after first colour stop"));
        }
        self.skip_inline_ws();
        let p0x = self.parse_signed_number()?;
        self.expect_char(b',')?;
        let p0y = self.parse_signed_number()?;
        self.skip_inline_ws();
        self.expect_char(b',')?;
        self.skip_inline_ws();
        let c1 = self.parse_color()?;
        self.skip_inline_ws();
        if !self.try_keyword("at") {
            return Err(ParseError::at(self.line, "expected `at <x>,<y>` after second colour stop"));
        }
        self.skip_inline_ws();
        let p1x = self.parse_signed_number()?;
        self.expect_char(b',')?;
        let p1y = self.parse_signed_number()?;
        self.skip_inline_ws();
        self.expect_char(b')')?;
        Ok(Fill::LinearGradient {
            c0,
            c1,
            p0: Point::new(p0x, p0y),
            p1: Point::new(p1x, p1y),
        })
    }

    fn parse_shape(&mut self) -> Result<Shape, ParseError> {
        let kind = self.parse_identifier()?;
        match kind.as_str() {
            "polygon" => {
                self.expect_char(b'(')?;
                let pts = self.parse_point_list()?;
                self.expect_char(b')')?;
                if pts.len() < 3 {
                    return Err(ParseError::at(
                        self.line,
                        format!("polygon needs at least 3 points (got {})", pts.len()),
                    ));
                }
                Ok(Shape::Polygon(pts))
            }
            "path" => {
                self.expect_char(b'(')?;
                let segs = self.parse_path_segments()?;
                self.expect_char(b')')?;
                if segs.is_empty() {
                    return Err(ParseError::at(self.line, "path needs at least one segment"));
                }
                if !matches!(segs.first(), Some(PathSeg::MoveTo(_))) {
                    return Err(ParseError::at(self.line, "path must start with `M x,y`"));
                }
                Ok(Shape::Path(segs))
            }
            other => Err(ParseError::at(
                self.line,
                format!("unknown shape `{other}` (v0.2 supports `polygon`, `path`)"),
            )),
        }
    }

    fn parse_path_segments(&mut self) -> Result<Vec<PathSeg>, ParseError> {
        let mut out = Vec::new();
        loop {
            self.skip_inline_ws();
            self.skip_trivia();
            if self.peek() == Some(b')') {
                break;
            }
            let op = match self.peek() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.bump();
                    c
                }
                _ => {
                    return Err(ParseError::at(
                        self.line,
                        "expected path command (M / L / C) or `)` to close path",
                    ));
                }
            };
            match op {
                b'M' | b'L' => {
                    self.skip_inline_ws();
                    let x = self.parse_signed_number()?;
                    self.expect_char(b',')?;
                    let y = self.parse_signed_number()?;
                    let p = Point::new(x, y);
                    out.push(if op == b'M' { PathSeg::MoveTo(p) } else { PathSeg::LineTo(p) });
                }
                b'C' => {
                    self.skip_inline_ws();
                    let c1x = self.parse_signed_number()?;
                    self.expect_char(b',')?;
                    let c1y = self.parse_signed_number()?;
                    self.skip_inline_ws();
                    let c2x = self.parse_signed_number()?;
                    self.expect_char(b',')?;
                    let c2y = self.parse_signed_number()?;
                    self.skip_inline_ws();
                    let px = self.parse_signed_number()?;
                    self.expect_char(b',')?;
                    let py = self.parse_signed_number()?;
                    out.push(PathSeg::CurveTo {
                        c1: Point::new(c1x, c1y),
                        c2: Point::new(c2x, c2y),
                        p: Point::new(px, py),
                    });
                }
                other => {
                    return Err(ParseError::at(
                        self.line,
                        format!("unknown path command `{}` (expected M / L / C)", other as char),
                    ));
                }
            }
        }
        Ok(out)
    }

    fn parse_point_list(&mut self) -> Result<Vec<Point>, ParseError> {
        let mut out = Vec::new();
        loop {
            self.skip_trivia();
            if self.peek() == Some(b')') {
                break;
            }
            let x = self.parse_signed_number()?;
            self.skip_inline_ws();
            let y = self.parse_signed_number()?;
            out.push(Point::new(x, y));
            self.skip_inline_ws();
            if self.peek() == Some(b',') {
                self.bump();
            } else {
                self.skip_trivia();
                if self.peek() != Some(b')') {
                    return Err(ParseError::at(
                        self.line,
                        "expected `,` between points or `)` to close polygon",
                    ));
                }
            }
        }
        Ok(out)
    }

    fn parse_color(&mut self) -> Result<Color, ParseError> {
        self.expect_char(b'#')?;
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_hexdigit() {
                self.bump();
            } else {
                break;
            }
        }
        let hex = &self.src[start..self.pos];
        match hex.len() {
            6 => Ok(Color::rgb(
                u8::from_str_radix(std::str::from_utf8(&hex[0..2]).unwrap(), 16).unwrap(),
                u8::from_str_radix(std::str::from_utf8(&hex[2..4]).unwrap(), 16).unwrap(),
                u8::from_str_radix(std::str::from_utf8(&hex[4..6]).unwrap(), 16).unwrap(),
            )),
            8 => Ok(Color::rgba(
                u8::from_str_radix(std::str::from_utf8(&hex[0..2]).unwrap(), 16).unwrap(),
                u8::from_str_radix(std::str::from_utf8(&hex[2..4]).unwrap(), 16).unwrap(),
                u8::from_str_radix(std::str::from_utf8(&hex[4..6]).unwrap(), 16).unwrap(),
                u8::from_str_radix(std::str::from_utf8(&hex[6..8]).unwrap(), 16).unwrap(),
            )),
            n => Err(ParseError::at(
                self.line,
                format!("expected #rrggbb or #rrggbbaa (got {n} hex digits)"),
            )),
        }
    }

    fn parse_identifier(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => self.bump(),
            _ => {
                return Err(ParseError::at(self.line, "expected identifier"));
            }
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.bump();
            } else {
                break;
            }
        }
        Ok(std::str::from_utf8(&self.src[start..self.pos]).unwrap().to_string())
    }

    fn parse_uint(&mut self) -> Result<u32, ParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.bump();
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(ParseError::at(self.line, "expected non-negative integer"));
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .unwrap()
            .parse::<u32>()
            .map_err(|e| ParseError::at(self.line, e.to_string()))
    }

    fn parse_signed_number(&mut self) -> Result<f32, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.bump();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == b'.' {
                self.bump();
            } else {
                break;
            }
        }
        if self.pos == start || (self.pos - start == 1 && self.src[start] == b'-') {
            return Err(ParseError::at(self.line, "expected number"));
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .unwrap()
            .parse::<f32>()
            .map_err(|e| ParseError::at(self.line, e.to_string()))
    }

    fn try_keyword(&mut self, kw: &str) -> bool {
        let bytes = kw.as_bytes();
        if self.pos + bytes.len() > self.src.len() {
            return false;
        }
        if &self.src[self.pos..self.pos + bytes.len()] != bytes {
            return false;
        }
        let next = self.src.get(self.pos + bytes.len()).copied();
        match next {
            Some(c) if c.is_ascii_alphanumeric() || c == b'_' => false,
            _ => {
                self.pos += bytes.len();
                true
            }
        }
    }

    fn expect_char(&mut self, c: u8) -> Result<(), ParseError> {
        match self.peek() {
            Some(actual) if actual == c => {
                self.bump();
                Ok(())
            }
            Some(actual) => Err(ParseError::at(
                self.line,
                format!("expected `{}` got `{}`", c as char, actual as char),
            )),
            None => Err(ParseError::at(self.line, format!("expected `{}` got EOF", c as char))),
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            while let Some(c) = self.peek() {
                match c {
                    b' ' | b'\t' | b'\r' => self.bump(),
                    b'\n' => {
                        self.line += 1;
                        self.bump();
                    }
                    b'#' if self.peek_after_hash_is_comment() => {
                        // Line comment.  `#` followed by alnum without a
                        // hex-string shape is treated as a comment so
                        // colours (#rrggbb) parse normally elsewhere.
                        while let Some(c) = self.peek() {
                            if c == b'\n' {
                                break;
                            }
                            self.bump();
                        }
                    }
                    _ => return,
                }
            }
            return;
        }
    }

    /// Heuristic: at the *statement* level, a `#` always begins a
    /// comment.  Colour `#rrggbb` literals only appear after `=` /
    /// `background`, and we only call `skip_trivia` outside the
    /// colour parser, so this distinction is safe in practice.
    fn peek_after_hash_is_comment(&self) -> bool {
        // Always true at statement level (we never call skip_trivia
        // while expecting a colour literal).
        true
    }

    fn skip_inline_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }
    fn bump(&mut self) {
        self.pos += 1;
    }
    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }
    fn peek_word_for_error(&self) -> String {
        let start = self.pos;
        let mut end = start;
        while end < self.src.len() {
            let c = self.src[end];
            if c == b'\n' || c == b' ' {
                break;
            }
            end += 1;
        }
        std::str::from_utf8(&self.src[start..end.min(start + 24)]).unwrap_or("?").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::write;

    #[test]
    fn round_trips_minimal_document() {
        let src = "canvas 100x60\nbackground #112233\n\nlet a = polygon(10 10, 50 10, 50 40, 10 40)\na.fill = #ff0000\n";
        let doc = parse(src).expect("parse");
        let out = write(&doc);
        assert_eq!(parse(&out).unwrap(), doc, "round-trip mismatch:\nfirst:\n{src}\nsecond:\n{out}");
    }

    #[test]
    fn parses_negative_coords() {
        let src = "canvas 100x100\nlet a = polygon(-5 10, 50 10, 50 40)\na.fill = #00ff00\n";
        let doc = parse(src).expect("parse");
        let Shape::Polygon(pts) = &doc.shapes[0].shape else { panic!("expected polygon"); };
        assert_eq!(pts[0], Point::new(-5.0, 10.0));
    }

    #[test]
    fn rejects_polygon_with_fewer_than_three_points() {
        let src = "canvas 100x100\nlet a = polygon(10 10, 50 10)\na.fill = #00ff00\n";
        assert!(parse(src).is_err());
    }

    #[test]
    fn skips_comments() {
        let src = "canvas 100x100\n# a comment\nlet a = polygon(0 0, 10 0, 10 10)\na.fill = #00ff00\n";
        let doc = parse(src).expect("parse");
        assert_eq!(doc.shapes.len(), 1);
    }

    #[test]
    fn round_trips_path_with_lines_and_curves() {
        let src = "canvas 200x200\n\nlet blob = path(M 50,50 L 150,50 C 180,60 180,140 150,150 L 50,150 C 20,140 20,60 50,50)\nblob.fill = #336699\n";
        let doc = parse(src).expect("parse path");
        let Shape::Path(segs) = &doc.shapes[0].shape else { panic!("expected path"); };
        assert_eq!(segs.len(), 5); // M L C L C
        assert!(matches!(segs[0], PathSeg::MoveTo(_)));
        assert!(matches!(segs[2], PathSeg::CurveTo { .. }));

        let out = write(&doc);
        let reparsed = parse(&out).expect("re-parse");
        assert_eq!(reparsed, doc, "round-trip mismatch:\n{out}");
    }

    #[test]
    fn rejects_path_without_initial_moveto() {
        let src = "canvas 100x100\nlet bad = path(L 10,10 L 50,50)\nbad.fill = #ff0000\n";
        assert!(parse(src).is_err());
    }
}
