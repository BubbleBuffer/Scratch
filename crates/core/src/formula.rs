//! The formula language of `PropKind::Formula` properties: a small
//! expression language over one row's properties.
//!
//! - Literals: numbers (`3`, `2.5`), strings (`"text"`, with `\"` and `\\`
//!   escapes), `true`, `false`.
//! - `prop("Name")`: a property of the row by display name (case-insensitive;
//!   `"Name"` or `"Title"` give the page title when no property has that
//!   name).
//! - Operators, loosest first: `cond ? a : b`, `or` / `||`, `and` / `&&`,
//!   `==` `!=`, `<` `<=` `>` `>=`, `+` `-`, `*` `/` `%`, `^` (right
//!   associative), unary `-`, `not` / `!`. `+` with a text on either side
//!   joins text.
//! - Functions: see [`call`]. Errors (unknown property or function, wrong
//!   types, division by zero) become the result's error message; they never
//!   panic.

use crate::markdown::{civil_from_ms, days_from_civil, iso8601, iso_date};

/// A value during evaluation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Empty,
    Num(f64),
    Str(String),
    Bool(bool),
    Date {
        start: i64,
        end: Option<i64>,
        has_time: bool,
    },
    List(Vec<Val>),
}

pub(crate) type EvalResult = std::result::Result<Val, String>;

/// What a formula reads from outside.
pub(crate) trait Env {
    /// The value of the property named `name` (an error if there is none).
    fn prop(&mut self, name: &str) -> EvalResult;
    /// The current time (ms since the epoch).
    fn now(&self) -> i64;
}

// ------------------------------------------------------------------ parsing

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
}

fn lex(src: &str) -> std::result::Result<Vec<Tok>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    const OPS: &[&str] = &[
        "==", "!=", "<=", ">=", "&&", "||", "+", "-", "*", "/", "%", "^", "<", ">", "!", "(", ")",
        ",", "?", ":",
    ];
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let s = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            if i < cs.len() && (cs[i] == 'e' || cs[i] == 'E') {
                let save = i;
                i += 1;
                if i < cs.len() && (cs[i] == '+' || cs[i] == '-') {
                    i += 1;
                }
                if i < cs.len() && cs[i].is_ascii_digit() {
                    while i < cs.len() && cs[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let t: String = cs[s..i].iter().collect();
            out.push(Tok::Num(
                t.parse().map_err(|_| format!("Invalid number {t}"))?,
            ));
        } else if c == '"' || c == '\u{201c}' || c == '\'' {
            let close = match c {
                '\u{201c}' => '\u{201d}',
                q => q,
            };
            i += 1;
            let mut s = String::new();
            loop {
                match cs.get(i) {
                    None => return Err("Missing closing quote".into()),
                    Some('\\') => {
                        match cs.get(i + 1) {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(x) => s.push(*x),
                            None => return Err("Missing closing quote".into()),
                        }
                        i += 2;
                    }
                    Some(x) if *x == close || (close == '"' && *x == '\u{201d}') => {
                        i += 1;
                        break;
                    }
                    Some(x) => {
                        s.push(*x);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else if c.is_alphabetic() || c == '_' {
            let s = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(cs[s..i].iter().collect()));
        } else {
            let rest: String = cs[i..cs.len().min(i + 2)].iter().collect();
            match OPS.iter().find(|o| rest.starts_with(**o)) {
                Some(o) => {
                    out.push(Tok::Op(o));
                    i += o.chars().count();
                }
                None => return Err(format!("Unexpected character {c}")),
            }
        }
    }
    Ok(out)
}

/// A parsed formula.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Lit(Val),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: u32,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(i)) if i.eq_ignore_ascii_case(w))
    }
    fn expect(&mut self, op: &str) -> std::result::Result<(), String> {
        if self.is_op(op) {
            self.pos += 1;
            Ok(())
        } else {
            Err(match self.peek() {
                Some(t) => format!("Expected {op} but found {}", tok_text(t)),
                None => format!("Expected {op} at the end"),
            })
        }
    }

    fn expr(&mut self) -> std::result::Result<Expr, String> {
        self.depth += 1;
        if self.depth > 200 {
            return Err("Formula is nested too deeply".into());
        }
        let c = self.or()?;
        let out = if self.is_op("?") {
            self.pos += 1;
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            Expr::Cond(Box::new(c), Box::new(a), Box::new(b))
        } else {
            c
        };
        self.depth -= 1;
        Ok(out)
    }

    fn or(&mut self) -> std::result::Result<Expr, String> {
        let mut l = self.and()?;
        while self.is_op("||") || self.is_word("or") {
            self.pos += 1;
            l = Expr::Binary("or", Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }

    fn and(&mut self) -> std::result::Result<Expr, String> {
        let mut l = self.eq()?;
        while self.is_op("&&") || self.is_word("and") {
            self.pos += 1;
            l = Expr::Binary("and", Box::new(l), Box::new(self.eq()?));
        }
        Ok(l)
    }

    fn binary_level(
        &mut self,
        ops: &[&'static str],
        next: fn(&mut Parser) -> std::result::Result<Expr, String>,
    ) -> std::result::Result<Expr, String> {
        let mut l = next(self)?;
        loop {
            let Some(op) = ops.iter().find(|o| self.is_op(o)) else {
                return Ok(l);
            };
            self.pos += 1;
            l = Expr::Binary(op, Box::new(l), Box::new(next(self)?));
        }
    }

    fn eq(&mut self) -> std::result::Result<Expr, String> {
        self.binary_level(&["==", "!="], Parser::cmp)
    }
    fn cmp(&mut self) -> std::result::Result<Expr, String> {
        self.binary_level(&["<=", ">=", "<", ">"], Parser::add)
    }
    fn add(&mut self) -> std::result::Result<Expr, String> {
        self.binary_level(&["+", "-"], Parser::mul)
    }
    fn mul(&mut self) -> std::result::Result<Expr, String> {
        self.binary_level(&["*", "/", "%"], Parser::unary)
    }

    fn unary(&mut self) -> std::result::Result<Expr, String> {
        if self.is_op("-") {
            self.pos += 1;
            return Ok(Expr::Unary("-", Box::new(self.unary()?)));
        }
        if self.is_op("+") {
            self.pos += 1;
            return self.unary();
        }
        if self.is_op("!") || self.is_word("not") {
            self.pos += 1;
            return Ok(Expr::Unary("not", Box::new(self.unary()?)));
        }
        self.pow()
    }

    fn pow(&mut self) -> std::result::Result<Expr, String> {
        let base = self.atom()?;
        if self.is_op("^") {
            self.pos += 1;
            let exp = self.unary()?;
            return Ok(Expr::Binary("^", Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> std::result::Result<Expr, String> {
        let t = self
            .peek()
            .cloned()
            .ok_or_else(|| "Unexpected end of formula".to_string())?;
        self.pos += 1;
        match t {
            Tok::Num(n) => Ok(Expr::Lit(Val::Num(n))),
            Tok::Str(s) => Ok(Expr::Lit(Val::Str(s))),
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                let lower = name.to_ascii_lowercase();
                if self.is_op("(") {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if !self.is_op(")") {
                        loop {
                            args.push(self.expr()?);
                            if self.is_op(",") {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(")")?;
                    return Ok(Expr::Call(lower, args));
                }
                match lower.as_str() {
                    "true" => Ok(Expr::Lit(Val::Bool(true))),
                    "false" => Ok(Expr::Lit(Val::Bool(false))),
                    _ => Err(format!("Unknown name {name}")),
                }
            }
            t => Err(format!("Unexpected {}", tok_text(&t))),
        }
    }
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Num(n) => format_num(*n),
        Tok::Str(s) => format!("\"{s}\""),
        Tok::Ident(s) => s.clone(),
        Tok::Op(o) => o.to_string(),
    }
}

/// Parses a formula. An empty formula is an empty value.
pub(crate) fn parse(src: &str) -> std::result::Result<Expr, String> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Ok(Expr::Lit(Val::Empty));
    }
    let mut p = Parser {
        toks,
        pos: 0,
        depth: 0,
    };
    let e = p.expr()?;
    if let Some(t) = p.peek() {
        return Err(format!("Unexpected {}", tok_text(t)));
    }
    Ok(e)
}

// ------------------------------------------------------------------ values

pub(crate) fn format_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        let s = format!("{:.10}", x);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// A value as text (what `format` and text joins produce).
pub(crate) fn to_text(v: &Val) -> String {
    match v {
        Val::Empty => String::new(),
        Val::Num(n) => format_num(*n),
        Val::Str(s) => s.clone(),
        Val::Bool(b) => b.to_string(),
        Val::Date {
            start,
            end,
            has_time,
        } => crate::markdown::date_text(*start, *end, *has_time),
        Val::List(items) => items.iter().map(to_text).collect::<Vec<_>>().join(", "),
    }
}

fn type_name(v: &Val) -> &'static str {
    match v {
        Val::Empty => "empty",
        Val::Num(_) => "number",
        Val::Str(_) => "text",
        Val::Bool(_) => "checkbox",
        Val::Date { .. } => "date",
        Val::List(_) => "list",
    }
}

fn num(v: &Val, what: &str) -> std::result::Result<f64, String> {
    match v {
        Val::Num(n) => Ok(*n),
        Val::Empty => Ok(0.0),
        o => Err(format!("{what} needs a number, not {}", type_name(o))),
    }
}

fn boolean(v: &Val, what: &str) -> std::result::Result<bool, String> {
    match v {
        Val::Bool(b) => Ok(*b),
        Val::Empty => Ok(false),
        o => Err(format!(
            "{what} needs a checkbox value, not {}",
            type_name(o)
        )),
    }
}

fn date(v: &Val, what: &str) -> std::result::Result<(i64, Option<i64>, bool), String> {
    match v {
        Val::Date {
            start,
            end,
            has_time,
        } => Ok((*start, *end, *has_time)),
        o => Err(format!("{what} needs a date, not {}", type_name(o))),
    }
}

fn is_empty(v: &Val) -> bool {
    match v {
        Val::Empty => true,
        Val::Str(s) => s.is_empty(),
        Val::List(l) => l.is_empty(),
        _ => false,
    }
}

fn equal(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Num(x), Val::Num(y)) => x == y,
        (Val::Date { start: x, .. }, Val::Date { start: y, .. }) => x == y,
        (x, y) if is_empty(x) || is_empty(y) => is_empty(x) && is_empty(y),
        (x, y) => x == y,
    }
}

fn compare(op: &str, a: &Val, b: &Val) -> EvalResult {
    use std::cmp::Ordering;
    let ord = match (a, b) {
        (Val::Empty, _) | (_, Val::Empty) => return Ok(Val::Bool(false)),
        (Val::Num(x), Val::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Val::Str(x), Val::Str(y)) => x.cmp(y),
        (Val::Date { start: x, .. }, Val::Date { start: y, .. }) => x.cmp(y),
        (Val::Bool(x), Val::Bool(y)) => x.cmp(y),
        (x, y) => {
            return Err(format!(
                "Cannot compare {} with {}",
                type_name(x),
                type_name(y)
            ))
        }
    };
    Ok(Val::Bool(match op {
        "<" => ord.is_lt(),
        "<=" => ord.is_le(),
        ">" => ord.is_gt(),
        _ => ord.is_ge(),
    }))
}

fn finite(x: f64) -> EvalResult {
    if x.is_finite() {
        Ok(Val::Num(x))
    } else {
        Err("The result is not a finite number".into())
    }
}

// ------------------------------------------------------------------ dates

const DAY: i64 = 86_400_000;

fn civil(ms: i64) -> (i64, i64, i64) {
    civil_from_ms(ms)
}

fn add_months(ms: i64, months: i64) -> i64 {
    let (y, m, d) = civil(ms);
    let tod = ms.rem_euclid(DAY);
    let total = y * 12 + (m - 1) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let days_in =
        days_from_civil(ny + i64::from(nm == 12), nm % 12 + 1, 1) - days_from_civil(ny, nm, 1);
    days_from_civil(ny, nm, d.min(days_in)) * DAY + tod
}

fn unit_of(u: &Val) -> std::result::Result<&'static str, String> {
    let s = match u {
        Val::Str(s) => s.trim().to_lowercase(),
        o => return Err(format!("A date unit is text, not {}", type_name(o))),
    };
    let s = s.strip_suffix('s').unwrap_or(&s).to_string();
    Ok(match s.as_str() {
        "year" => "years",
        "quarter" => "quarters",
        "month" => "months",
        "week" => "weeks",
        "day" => "days",
        "hour" => "hours",
        "minute" => "minutes",
        "second" => "seconds",
        "millisecond" => "milliseconds",
        _ => return Err(format!("Unknown date unit {s}")),
    })
}

fn shift(ms: i64, n: f64, unit: &str) -> i64 {
    let n_i = n.trunc() as i64;
    match unit {
        "years" => add_months(ms, n_i * 12),
        "quarters" => add_months(ms, n_i * 3),
        "months" => add_months(ms, n_i),
        "weeks" => ms + (n * 7.0 * DAY as f64) as i64,
        "days" => ms + (n * DAY as f64) as i64,
        "hours" => ms + (n * 3_600_000.0) as i64,
        "minutes" => ms + (n * 60_000.0) as i64,
        "seconds" => ms + (n * 1000.0) as i64,
        _ => ms + n as i64,
    }
}

fn between(a: i64, b: i64, unit: &str) -> f64 {
    let diff = (a - b) as f64;
    let months = || {
        let (ya, ma, _) = civil(a);
        let (yb, mb, _) = civil(b);
        let mut m = (ya * 12 + ma) - (yb * 12 + mb);
        // A month is complete only once the day of month is reached.
        if m > 0 && add_months(b, m) > a {
            m -= 1;
        } else if m < 0 && add_months(b, m) < a {
            m += 1;
        }
        m
    };
    match unit {
        "years" => (months() / 12) as f64,
        "quarters" => (months() / 3) as f64,
        "months" => months() as f64,
        "weeks" => (diff / (7 * DAY) as f64).trunc(),
        "days" => (diff / DAY as f64).trunc(),
        "hours" => (diff / 3_600_000.0).trunc(),
        "minutes" => (diff / 60_000.0).trunc(),
        "seconds" => (diff / 1000.0).trunc(),
        _ => diff,
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// 1 (Monday) to 7 (Sunday).
fn weekday(ms: i64) -> i64 {
    // 1970-01-01 was a Thursday.
    (ms.div_euclid(DAY) + 3).rem_euclid(7) + 1
}

/// Formats a date with moment-style tokens: YYYY YY MMMM MMM MM M DD D
/// dddd ddd HH H hh h mm ss A; text in `[brackets]` is kept as is.
fn format_date(ms: i64, fmt: &str) -> String {
    let (y, m, d) = civil(ms);
    let sod = ms.rem_euclid(DAY) / 1000;
    let (hh, mi, ss) = (sod / 3600, (sod / 60) % 60, sod % 60);
    let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
    let wd = weekday(ms) as usize - 1;
    const TOKENS: &[&str] = &[
        "YYYY", "YY", "MMMM", "MMM", "MM", "M", "DD", "D", "dddd", "ddd", "HH", "H", "hh", "h",
        "mm", "ss", "A",
    ];
    let cs: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '[' {
            if let Some(j) = cs[i..].iter().position(|c| *c == ']') {
                out.extend(&cs[i + 1..i + j]);
                i += j + 1;
                continue;
            }
        }
        let rest: String = cs[i..cs.len().min(i + 4)].iter().collect();
        match TOKENS.iter().find(|t| rest.starts_with(**t)) {
            Some(t) => {
                out.push_str(&match *t {
                    "YYYY" => format!("{y:04}"),
                    "YY" => format!("{:02}", y.rem_euclid(100)),
                    "MMMM" => MONTHS[(m - 1) as usize].to_string(),
                    "MMM" => MONTHS[(m - 1) as usize][..3].to_string(),
                    "MM" => format!("{m:02}"),
                    "M" => m.to_string(),
                    "DD" => format!("{d:02}"),
                    "D" => d.to_string(),
                    "dddd" => WEEKDAYS[wd].to_string(),
                    "ddd" => WEEKDAYS[wd][..3].to_string(),
                    "HH" => format!("{hh:02}"),
                    "H" => hh.to_string(),
                    "hh" => format!("{h12:02}"),
                    "h" => h12.to_string(),
                    "mm" => format!("{mi:02}"),
                    "ss" => format!("{ss:02}"),
                    _ => (if hh < 12 { "AM" } else { "PM" }).to_string(),
                });
                i += t.len();
            }
            None => {
                out.push(cs[i]);
                i += 1;
            }
        }
    }
    out
}

// ------------------------------------------------------------------ evaluation

/// Evaluates a parsed formula.
pub(crate) fn eval(e: &Expr, env: &mut dyn Env) -> EvalResult {
    match e {
        Expr::Lit(v) => Ok(v.clone()),
        Expr::Unary(op, x) => {
            let v = eval(x, env)?;
            match *op {
                "-" => finite(-num(&v, "-")?),
                _ => Ok(Val::Bool(!boolean(&v, "not")?)),
            }
        }
        Expr::Cond(c, a, b) => {
            if boolean(&eval(c, env)?, "?")? {
                eval(a, env)
            } else {
                eval(b, env)
            }
        }
        Expr::Binary(op, l, r) => {
            match *op {
                "and" => {
                    return Ok(Val::Bool(
                        boolean(&eval(l, env)?, "and")? && boolean(&eval(r, env)?, "and")?,
                    ))
                }
                "or" => {
                    return Ok(Val::Bool(
                        boolean(&eval(l, env)?, "or")? || boolean(&eval(r, env)?, "or")?,
                    ))
                }
                _ => {}
            }
            let a = eval(l, env)?;
            let b = eval(r, env)?;
            binary(op, &a, &b)
        }
        Expr::Call(name, args) => call(name, args, env),
    }
}

fn binary(op: &str, a: &Val, b: &Val) -> EvalResult {
    match op {
        "+" => match (a, b) {
            (Val::Str(_), _) | (_, Val::Str(_)) => {
                Ok(Val::Str(format!("{}{}", to_text(a), to_text(b))))
            }
            (Val::Empty, Val::Empty) => Ok(Val::Empty),
            _ => finite(num(a, "+")? + num(b, "+")?),
        },
        "-" => finite(num(a, "-")? - num(b, "-")?),
        "*" => finite(num(a, "*")? * num(b, "*")?),
        "/" => {
            let d = num(b, "/")?;
            if d == 0.0 {
                return Err("Division by zero".into());
            }
            finite(num(a, "/")? / d)
        }
        "%" => {
            let d = num(b, "%")?;
            if d == 0.0 {
                return Err("Division by zero".into());
            }
            finite(num(a, "%")? % d)
        }
        "^" => finite(num(a, "^")?.powf(num(b, "^")?)),
        "==" => Ok(Val::Bool(equal(a, b))),
        "!=" => Ok(Val::Bool(!equal(a, b))),
        _ => compare(op, a, b),
    }
}

fn arity(name: &str, args: &[Expr], min: usize, max: usize) -> std::result::Result<(), String> {
    if args.len() < min || args.len() > max {
        let want = if min == max {
            format!("{min}")
        } else if max == usize::MAX {
            format!("at least {min}")
        } else {
            format!("{min} to {max}")
        };
        return Err(format!(
            "{name}() takes {want} argument{}, not {}",
            if max == 1 && min == 1 { "" } else { "s" },
            args.len()
        ));
    }
    Ok(())
}

fn text_of(v: &Val) -> String {
    to_text(v)
}

fn flatten(vals: Vec<Val>) -> Vec<Val> {
    let mut out = Vec::new();
    for v in vals {
        match v {
            Val::List(l) => out.extend(flatten(l)),
            v => out.push(v),
        }
    }
    out
}

/// The built-in functions.
fn call(name: &str, args: &[Expr], env: &mut dyn Env) -> EvalResult {
    // Lazy ones first.
    match name {
        "if" => {
            arity(name, args, 3, 3)?;
            return if boolean(&eval(&args[0], env)?, "if")? {
                eval(&args[1], env)
            } else {
                eval(&args[2], env)
            };
        }
        "ifs" => {
            if args.len() < 3 || args.len().is_multiple_of(2) {
                return Err("ifs() takes condition, value pairs and a default".into());
            }
            for pair in args[..args.len() - 1].chunks(2) {
                if boolean(&eval(&pair[0], env)?, "ifs")? {
                    return eval(&pair[1], env);
                }
            }
            return eval(&args[args.len() - 1], env);
        }
        "prop" => {
            arity(name, args, 1, 1)?;
            return match eval(&args[0], env)? {
                Val::Str(s) => env.prop(&s),
                o => Err(format!(
                    "prop() takes a property name in quotes, not {}",
                    type_name(&o)
                )),
            };
        }
        _ => {}
    }
    let v: Vec<Val> = args
        .iter()
        .map(|a| eval(a, env))
        .collect::<std::result::Result<_, _>>()?;
    let a = |i: usize| -> &Val { &v[i] };
    match name {
        "and" | "or" => {
            arity(name, args, 1, usize::MAX)?;
            let mut acc = name == "and";
            for x in &v {
                let b = boolean(x, name)?;
                acc = if name == "and" { acc && b } else { acc || b };
            }
            Ok(Val::Bool(acc))
        }
        "not" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Bool(!boolean(a(0), name)?))
        }
        "empty" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Bool(is_empty(a(0))))
        }
        "concat" => Ok(Val::Str(v.iter().map(text_of).collect())),
        "join" => {
            arity(name, args, 2, 2)?;
            let sep = text_of(a(1));
            let items = match a(0) {
                Val::List(l) => l.clone(),
                Val::Empty => Vec::new(),
                o => vec![o.clone()],
            };
            Ok(Val::Str(
                items.iter().map(text_of).collect::<Vec<_>>().join(&sep),
            ))
        }
        "length" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Num(match a(0) {
                Val::List(l) => l.len() as f64,
                Val::Empty => 0.0,
                o => text_of(o).chars().count() as f64,
            }))
        }
        "lower" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Str(text_of(a(0)).to_lowercase()))
        }
        "upper" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Str(text_of(a(0)).to_uppercase()))
        }
        "trim" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Str(text_of(a(0)).trim().to_string()))
        }
        "contains" | "includes" => {
            arity(name, args, 2, 2)?;
            Ok(Val::Bool(match a(0) {
                Val::List(l) => l
                    .iter()
                    .any(|x| equal(x, a(1)) || text_of(x) == text_of(a(1))),
                o => text_of(o).contains(&text_of(a(1))),
            }))
        }
        "startswith" | "endswith" => {
            arity(name, args, 2, 2)?;
            let (s, p) = (text_of(a(0)), text_of(a(1)));
            Ok(Val::Bool(if name == "startswith" {
                s.starts_with(&p)
            } else {
                s.ends_with(&p)
            }))
        }
        "replace" | "replaceall" => {
            arity(name, args, 3, 3)?;
            let (s, from, to) = (text_of(a(0)), text_of(a(1)), text_of(a(2)));
            if from.is_empty() {
                return Ok(Val::Str(s));
            }
            Ok(Val::Str(if name == "replace" {
                s.replacen(&from, &to, 1)
            } else {
                s.replace(&from, &to)
            }))
        }
        "slice" | "substring" => {
            arity(name, args, 2, 3)?;
            let cs: Vec<char> = text_of(a(0)).chars().collect();
            let n = cs.len() as i64;
            let clamp = |x: f64| -> usize {
                let x = x.trunc() as i64;
                let x = if x < 0 { n + x } else { x };
                x.clamp(0, n) as usize
            };
            let s = clamp(num(a(1), name)?);
            let e = if v.len() > 2 {
                clamp(num(a(2), name)?)
            } else {
                n as usize
            };
            Ok(Val::Str(if s < e {
                cs[s..e].iter().collect()
            } else {
                String::new()
            }))
        }
        "repeat" => {
            arity(name, args, 2, 2)?;
            let n = num(a(1), name)?.clamp(0.0, 10_000.0) as usize;
            Ok(Val::Str(text_of(a(0)).repeat(n)))
        }
        "format" | "tostring" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Str(text_of(a(0))))
        }
        "tonumber" => {
            arity(name, args, 1, 1)?;
            match a(0) {
                Val::Num(n) => Ok(Val::Num(*n)),
                Val::Bool(b) => Ok(Val::Num(if *b { 1.0 } else { 0.0 })),
                Val::Date { start, .. } => Ok(Val::Num(*start as f64)),
                Val::Empty => Ok(Val::Empty),
                o => match text_of(o).trim().replace(',', "").parse::<f64>() {
                    Ok(x) if x.is_finite() => Ok(Val::Num(x)),
                    _ => Ok(Val::Empty),
                },
            }
        }
        "abs" | "floor" | "ceil" | "sqrt" | "sign" | "exp" | "ln" | "log10" | "log2" => {
            arity(name, args, 1, 1)?;
            let x = num(a(0), name)?;
            if matches!(name, "sqrt" | "ln" | "log10" | "log2") && x < 0.0 {
                return Err(format!("{name}() of a negative number"));
            }
            finite(match name {
                "abs" => x.abs(),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "sqrt" => x.sqrt(),
                "sign" => {
                    if x == 0.0 {
                        0.0
                    } else {
                        x.signum()
                    }
                }
                "exp" => x.exp(),
                "ln" => x.ln(),
                "log10" => x.log10(),
                _ => x.log2(),
            })
        }
        "round" => {
            arity(name, args, 1, 2)?;
            let x = num(a(0), name)?;
            let places = if v.len() > 1 {
                num(a(1), name)?.clamp(-15.0, 15.0) as i32
            } else {
                0
            };
            let f = 10f64.powi(places);
            finite((x * f).round() / f)
        }
        "pow" => {
            arity(name, args, 2, 2)?;
            finite(num(a(0), name)?.powf(num(a(1), name)?))
        }
        "mod" => {
            arity(name, args, 2, 2)?;
            binary("%", a(0), a(1))
        }
        "min" | "max" | "sum" | "mean" | "average" | "median" => {
            let items: Vec<f64> = flatten(v.clone())
                .iter()
                .filter(|x| !is_empty(x))
                .map(|x| num(x, name))
                .collect::<std::result::Result<_, _>>()?;
            if items.is_empty() {
                return Ok(if name == "sum" {
                    Val::Num(0.0)
                } else {
                    Val::Empty
                });
            }
            finite(match name {
                "min" => items.iter().cloned().fold(f64::INFINITY, f64::min),
                "max" => items.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                "sum" => items.iter().sum(),
                "median" => {
                    let mut s = items.clone();
                    s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                    let n = s.len();
                    if n % 2 == 1 {
                        s[n / 2]
                    } else {
                        (s[n / 2 - 1] + s[n / 2]) / 2.0
                    }
                }
                _ => items.iter().sum::<f64>() / items.len() as f64,
            })
        }
        "now" => {
            arity(name, args, 0, 0)?;
            Ok(Val::Date {
                start: env.now(),
                end: None,
                has_time: true,
            })
        }
        "today" => {
            arity(name, args, 0, 0)?;
            Ok(Val::Date {
                start: env.now().div_euclid(DAY) * DAY,
                end: None,
                has_time: false,
            })
        }
        "dateadd" | "datesubtract" => {
            arity(name, args, 3, 3)?;
            let (s, e, t) = date(a(0), name)?;
            let n = num(a(1), name)? * if name == "datesubtract" { -1.0 } else { 1.0 };
            let unit = unit_of(a(2))?;
            Ok(Val::Date {
                start: shift(s, n, unit),
                end: e.map(|e| shift(e, n, unit)),
                has_time: t || matches!(unit, "hours" | "minutes" | "seconds" | "milliseconds"),
            })
        }
        "datebetween" => {
            arity(name, args, 3, 3)?;
            let (x, _, _) = date(a(0), name)?;
            let (y, _, _) = date(a(1), name)?;
            finite(between(x, y, unit_of(a(2))?))
        }
        "formatdate" => {
            arity(name, args, 1, 2)?;
            let (s, _, t) = date(a(0), name)?;
            let fmt = if v.len() > 1 {
                text_of(a(1))
            } else if t {
                "MMMM D, YYYY h:mm A".into()
            } else {
                "MMMM D, YYYY".into()
            };
            Ok(Val::Str(format_date(s, &fmt)))
        }
        "parsedate" => {
            arity(name, args, 1, 1)?;
            match crate::import::parse_date_text(&text_of(a(0))) {
                Some(crate::types::PropValue::Date {
                    start_ms,
                    end_ms,
                    has_time,
                }) => Ok(Val::Date {
                    start: start_ms,
                    end: end_ms,
                    has_time,
                }),
                _ => Err(format!("Cannot read {:?} as a date", text_of(a(0)))),
            }
        }
        "start" | "datestart" | "end" | "dateend" => {
            arity(name, args, 1, 1)?;
            let (s, e, t) = date(a(0), name)?;
            let at = if name.ends_with("start") || name == "start" {
                s
            } else {
                e.unwrap_or(s)
            };
            Ok(Val::Date {
                start: at,
                end: None,
                has_time: t,
            })
        }
        "year" | "month" | "date" | "day" | "week" | "hour" | "minute" | "timestamp" => {
            arity(name, args, 1, 1)?;
            let (s, _, _) = date(a(0), name)?;
            let (y, m, d) = civil(s);
            let sod = s.rem_euclid(DAY) / 1000;
            Ok(Val::Num(match name {
                "year" => y as f64,
                "month" => m as f64,
                "date" => d as f64,
                "day" => weekday(s) as f64,
                "week" => {
                    // ISO week number.
                    let wd = weekday(s);
                    let thursday = s.div_euclid(DAY) - (wd - 4);
                    let (ty, _, _) = civil(thursday * DAY);
                    ((thursday - days_from_civil(ty, 1, 1)) / 7 + 1) as f64
                }
                "hour" => (sod / 3600) as f64,
                "minute" => ((sod / 60) % 60) as f64,
                _ => s as f64,
            }))
        }
        "fromtimestamp" => {
            arity(name, args, 1, 1)?;
            Ok(Val::Date {
                start: num(a(0), name)? as i64,
                end: None,
                has_time: true,
            })
        }
        "daterange" => {
            arity(name, args, 2, 2)?;
            let (s, _, t1) = date(a(0), name)?;
            let (e, _, t2) = date(a(1), name)?;
            if e < s {
                return Err("dateRange() ends before it starts".into());
            }
            Ok(Val::Date {
                start: s,
                end: Some(e),
                has_time: t1 || t2,
            })
        }
        _ => Err(format!("Unknown function {name}()")),
    }
}

/// `2026-10-01T08:00:00.000Z` / `2026-10-01` for debugging and tests.
#[allow(dead_code)]
pub(crate) fn date_debug(ms: i64, has_time: bool) -> String {
    if has_time {
        iso8601(ms)
    } else {
        iso_date(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Map(HashMap<String, Val>);
    impl Env for Map {
        fn prop(&mut self, name: &str) -> EvalResult {
            self.0
                .get(&name.to_lowercase())
                .cloned()
                .ok_or_else(|| format!("No property named {name}"))
        }
        fn now(&self) -> i64 {
            // 2026-10-01T12:00:00Z
            days_from_civil(2026, 10, 1) * DAY + 12 * 3_600_000
        }
    }

    fn run(src: &str) -> EvalResult {
        let mut env = Map(HashMap::from([
            ("price".to_string(), Val::Num(12.5)),
            ("qty".to_string(), Val::Num(4.0)),
            ("name".to_string(), Val::Str("Widget".into())),
            ("done".to_string(), Val::Bool(true)),
            ("none".to_string(), Val::Empty),
            (
                "due".to_string(),
                Val::Date {
                    start: days_from_civil(2026, 1, 31) * DAY,
                    end: None,
                    has_time: false,
                },
            ),
            (
                "tags".to_string(),
                Val::List(vec![Val::Str("a".into()), Val::Str("b".into())]),
            ),
        ]));
        eval(&parse(src)?, &mut env)
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(run("1 + 2 * 3"), Ok(Val::Num(7.0)));
        assert_eq!(run("(1 + 2) * 3"), Ok(Val::Num(9.0)));
        assert_eq!(run("2 ^ 3 ^ 2"), Ok(Val::Num(512.0)));
        assert_eq!(run("-2 ^ 2"), Ok(Val::Num(-4.0)));
        assert_eq!(run("7 % 4"), Ok(Val::Num(3.0)));
        assert_eq!(run("prop(\"Price\") * prop(\"Qty\")"), Ok(Val::Num(50.0)));
        assert_eq!(run("round(10 / 3, 2)"), Ok(Val::Num(3.33)));
        assert_eq!(run("prop(\"None\") + 1"), Ok(Val::Num(1.0)));
        assert_eq!(run("max(3, 9, 2) - min(4, 1)"), Ok(Val::Num(8.0)));
        assert_eq!(run(""), Ok(Val::Empty));
    }

    #[test]
    fn text_comparison_and_if() {
        assert_eq!(run("\"n: \" + 3"), Ok(Val::Str("n: 3".into())));
        assert_eq!(
            run("concat(prop(\"Name\"), \"-\", upper(\"x\"))"),
            Ok(Val::Str("Widget-X".into()))
        );
        assert_eq!(run("length(prop(\"Name\"))"), Ok(Val::Num(6.0)));
        assert_eq!(run("prop(\"Price\") > 10"), Ok(Val::Bool(true)));
        assert_eq!(
            run("prop(\"Price\") == 12.5 and not prop(\"Done\")"),
            Ok(Val::Bool(false))
        );
        assert_eq!(
            run("if(prop(\"Done\"), \"yes\", \"no\")"),
            Ok(Val::Str("yes".into()))
        );
        assert_eq!(run("prop(\"Qty\") >= 4 ? 1 : 2"), Ok(Val::Num(1.0)));
        assert_eq!(run("contains(prop(\"Tags\"), \"b\")"), Ok(Val::Bool(true)));
        assert_eq!(run("length(prop(\"Tags\"))"), Ok(Val::Num(2.0)));
        assert_eq!(run("empty(prop(\"None\"))"), Ok(Val::Bool(true)));
        assert_eq!(
            run("replaceAll(\"a-b-c\", \"-\", \"+\")"),
            Ok(Val::Str("a+b+c".into()))
        );
        assert_eq!(run("slice(\"Widget\", 1, 3)"), Ok(Val::Str("id".into())));
        assert_eq!(run("toNumber(\"1,200.5\")"), Ok(Val::Num(1200.5)));
        assert_eq!(run("format(true)"), Ok(Val::Str("true".into())));
    }

    #[test]
    fn dates() {
        let d = |y, m, dd| days_from_civil(y, m, dd) * DAY;
        assert_eq!(
            run("dateAdd(prop(\"Due\"), 1, \"months\")"),
            Ok(Val::Date {
                start: d(2026, 2, 28),
                end: None,
                has_time: false
            })
        );
        assert_eq!(
            run("dateSubtract(prop(\"Due\"), 2, \"days\")"),
            Ok(Val::Date {
                start: d(2026, 1, 29),
                end: None,
                has_time: false
            })
        );
        assert_eq!(
            run("dateBetween(today(), prop(\"Due\"), \"days\")"),
            Ok(Val::Num(243.0))
        );
        assert_eq!(
            run("dateBetween(today(), prop(\"Due\"), \"months\")"),
            Ok(Val::Num(8.0))
        );
        assert_eq!(
            run("formatDate(prop(\"Due\"), \"YYYY-MM-DD ddd\")"),
            Ok(Val::Str("2026-01-31 Sat".into()))
        );
        assert_eq!(
            run("formatDate(now(), \"MMM D, h:mm A\")"),
            Ok(Val::Str("Oct 1, 12:00 PM".into()))
        );
        assert_eq!(run("year(prop(\"Due\"))"), Ok(Val::Num(2026.0)));
        assert_eq!(run("month(prop(\"Due\"))"), Ok(Val::Num(1.0)));
        assert_eq!(run("date(prop(\"Due\"))"), Ok(Val::Num(31.0)));
        assert_eq!(run("day(prop(\"Due\"))"), Ok(Val::Num(6.0)));
        assert_eq!(run("prop(\"Due\") < now()"), Ok(Val::Bool(true)));
    }

    #[test]
    fn errors() {
        assert_eq!(run("1 / 0"), Err("Division by zero".into()));
        assert_eq!(
            run("prop(\"Missing\")"),
            Err("No property named Missing".into())
        );
        assert_eq!(run("nope(1)"), Err("Unknown function nope()".into()));
        assert!(run("1 +").is_err());
        assert!(run("(1 + 2").is_err());
        assert!(run("\"open").is_err());
        assert!(run("1 2").is_err());
        assert!(run("prop(\"Name\") * 2").is_err());
        assert!(run("prop(\"Name\") < 3").is_err());
        assert!(run("if(1, 2, 3)").is_err());
        assert!(run("if(true, 2)").is_err());
        assert!(run("dateAdd(prop(\"Due\"), 1, \"fortnights\")").is_err());
        assert!(run("sqrt(-1)").is_err());
        assert!(run("10 ^ 400").is_err());
        assert!(run("#").is_err());
        let deep = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        assert!(run(&deep).is_err());
    }
}
