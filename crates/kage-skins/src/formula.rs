//! Fórmulas do Rainmeter: `(340*#Scale#)`, `(a > b ? 1 : 0)`, `Round(x, 2)`.
//!
//! Operadores: `+ - * / % ^`, comparações `< > <= >= = <>`, `&& ||`, `!`, ternário `? :`.
//! Funções: Abs, Sgn, Sqrt, Ln, Log, Exp, Sin, Cos, Tan, ASin, ACos, ATan, ATan2, Ceil, Floor,
//! Round, Trunc, Frac, Min, Max, Clamp. Constantes: PI, E. Números decimais, `0x` e `0b`.
//! Nomes desconhecidos vão para `resolve` (o valor numérico de uma medida, por exemplo).

/// Valor de uma opção numérica: entre parênteses é fórmula, senão número simples.
pub fn number(value: &str) -> Option<f64> {
    let v = value.trim();
    if v.starts_with('(') { eval(v).ok() } else { v.parse().ok() }
}

pub fn eval(expr: &str) -> Result<f64, String> {
    eval_with(expr, &|_| None)
}

pub fn eval_with(expr: &str, resolve: &dyn Fn(&str) -> Option<f64>) -> Result<f64, String> {
    let tokens = tokenize(expr)?;
    let mut parser = Parser { tokens, pos: 0, resolve };
    let value = parser.ternary()?;
    if parser.pos != parser.tokens.len() {
        return Err(format!("sobrou texto na fórmula: {expr}"));
    }
    Ok(value)
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(&'static str),
}

fn tokenize(expr: &str) -> Result<Vec<Token>, String> {
    const OPS: [&str; 20] = [
        "<=", ">=", "<>", "&&", "||", "+", "-", "*", "/", "%", "^", "(", ")", ",", "?", ":", "<", ">", "=", "!",
    ];
    let chars: Vec<char> = expr.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let start = i;
            if c == '0' && matches!(chars.get(i + 1), Some('x' | 'X' | 'b' | 'B')) {
                let radix = if matches!(chars[i + 1], 'x' | 'X') { 16 } else { 2 };
                i += 2;
                let digits_start = i;
                while i < chars.len() && chars[i].is_digit(radix) {
                    i += 1;
                }
                let digits: String = chars[digits_start..i].iter().collect();
                let value = u64::from_str_radix(&digits, radix).map_err(|_| format!("número inválido em {expr}"))?;
                tokens.push(Token::Num(value as f64));
                continue;
            }
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            tokens.push(Token::Num(text.parse().map_err(|_| format!("número inválido: {text}"))?));
        } else if c.is_alphabetic() || c == '_' || c == '@' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '@' | '.')) {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
        } else {
            let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let op = OPS.iter().find(|op| rest.starts_with(**op)).ok_or(format!("símbolo inesperado '{c}' em {expr}"))?;
            tokens.push(Token::Op(op));
            i += op.chars().count();
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    resolve: &'a dyn Fn(&str) -> Option<f64>,
}

fn truth(v: f64) -> bool {
    v != 0.0
}

fn flag(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

impl Parser<'_> {
    fn peek_op(&self) -> Option<&'static str> {
        match self.tokens.get(self.pos) {
            Some(Token::Op(op)) => Some(op),
            _ => None,
        }
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.peek_op() == Some(op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        if self.eat(op) { Ok(()) } else { Err(format!("esperava '{op}'")) }
    }

    fn ternary(&mut self) -> Result<f64, String> {
        let cond = self.or()?;
        if self.eat("?") {
            let yes = self.ternary()?;
            self.expect(":")?;
            let no = self.ternary()?;
            return Ok(if truth(cond) { yes } else { no });
        }
        Ok(cond)
    }

    fn or(&mut self) -> Result<f64, String> {
        let mut v = self.and()?;
        while self.eat("||") {
            let r = self.and()?;
            v = flag(truth(v) || truth(r));
        }
        Ok(v)
    }

    fn and(&mut self) -> Result<f64, String> {
        let mut v = self.equality()?;
        while self.eat("&&") {
            let r = self.equality()?;
            v = flag(truth(v) && truth(r));
        }
        Ok(v)
    }

    fn equality(&mut self) -> Result<f64, String> {
        let mut v = self.comparison()?;
        loop {
            if self.eat("=") {
                v = flag(v == self.comparison()?);
            } else if self.eat("<>") {
                v = flag(v != self.comparison()?);
            } else {
                return Ok(v);
            }
        }
    }

    fn comparison(&mut self) -> Result<f64, String> {
        let mut v = self.additive()?;
        loop {
            let op = match self.peek_op() {
                Some(op @ ("<" | ">" | "<=" | ">=")) => op,
                _ => return Ok(v),
            };
            self.pos += 1;
            let r = self.additive()?;
            v = flag(match op {
                "<" => v < r,
                ">" => v > r,
                "<=" => v <= r,
                _ => v >= r,
            });
        }
    }

    fn additive(&mut self) -> Result<f64, String> {
        let mut v = self.multiplicative()?;
        loop {
            if self.eat("+") {
                v += self.multiplicative()?;
            } else if self.eat("-") {
                v -= self.multiplicative()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn multiplicative(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat("*") {
                v *= self.unary()?;
            } else if self.eat("/") {
                let r = self.unary()?;
                v = if r == 0.0 { 0.0 } else { v / r };
            } else if self.eat("%") {
                let r = self.unary()?;
                v = if r == 0.0 { 0.0 } else { v % r };
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.eat("-") {
            return Ok(-self.unary()?);
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("!") {
            return Ok(flag(!truth(self.unary()?)));
        }
        self.power()
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.primary()?;
        if self.eat("^") {
            // Associativa à direita: 2^3^2 = 2^9.
            return Ok(base.powf(self.unary()?));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<f64, String> {
        match self.tokens.get(self.pos).cloned() {
            Some(Token::Num(n)) => {
                self.pos += 1;
                Ok(n)
            }
            Some(Token::Op("(")) => {
                self.pos += 1;
                let v = self.ternary()?;
                self.expect(")")?;
                Ok(v)
            }
            Some(Token::Name(name)) => {
                self.pos += 1;
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    return call(&name, &args);
                }
                match name.to_ascii_lowercase().as_str() {
                    "pi" => Ok(std::f64::consts::PI),
                    "e" => Ok(std::f64::consts::E),
                    _ => (self.resolve)(&name).ok_or(format!("nome desconhecido: {name}")),
                }
            }
            other => Err(format!("esperava um valor, achei {other:?}")),
        }
    }
}

fn call(name: &str, a: &[f64]) -> Result<f64, String> {
    let arg = |i: usize| a.get(i).copied().ok_or(format!("{name} precisa de mais argumentos"));
    Ok(match name.to_ascii_lowercase().as_str() {
        "abs" => arg(0)?.abs(),
        "sgn" => arg(0)?.signum() * flag(arg(0)? != 0.0),
        "sqrt" => arg(0)?.sqrt(),
        "ln" => arg(0)?.ln(),
        "log" => arg(0)?.log10(),
        "exp" => arg(0)?.exp(),
        "sin" => arg(0)?.sin(),
        "cos" => arg(0)?.cos(),
        "tan" => arg(0)?.tan(),
        "asin" => arg(0)?.asin(),
        "acos" => arg(0)?.acos(),
        "atan" => arg(0)?.atan(),
        "atan2" => arg(0)?.atan2(arg(1)?),
        "ceil" => arg(0)?.ceil(),
        "floor" => arg(0)?.floor(),
        "trunc" => arg(0)?.trunc(),
        "frac" => arg(0)?.fract(),
        "round" => {
            let factor = 10f64.powi(a.get(1).copied().unwrap_or(0.0) as i32);
            (arg(0)? * factor).round() / factor
        }
        "min" => arg(0)?.min(arg(1)?),
        "max" => arg(0)?.max(arg(1)?),
        "clamp" => arg(0)?.clamp(arg(1)?, arg(2)?),
        _ => return Err(format!("função desconhecida: {name}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert!(close(eval("(340*0.86)").unwrap(), 292.4));
        assert!(close(eval("2 + 3 * 4").unwrap(), 14.0));
        assert!(close(eval("(2 + 3) * 4").unwrap(), 20.0));
        assert!(close(eval("2^3^2").unwrap(), 512.0));
        assert!(close(eval("-2^2").unwrap(), -4.0));
        assert!(close(eval("7 % 3").unwrap(), 1.0));
        assert!(close(eval("1/0").unwrap(), 0.0), "divisão por zero vira 0, como no Rainmeter");
    }

    #[test]
    fn mond_scroll_formula() {
        // A fórmula de rolagem da Mond: (Scale - incremento < 0.5 ? 0.5 : Scale - incremento).
        assert!(close(eval("(0.86-0.1 < 0.5 ? 0.5 : 0.86-0.1)").unwrap(), 0.76));
        assert!(close(eval("(0.55-0.1 < 0.5 ? 0.5 : 0.55-0.1)").unwrap(), 0.5));
    }

    #[test]
    fn comparisons_logic_and_functions() {
        assert!(close(eval("(3 >= 3) && (2 <> 1)").unwrap(), 1.0));
        assert!(close(eval("!(1 = 1) || 0").unwrap(), 0.0));
        assert!(close(eval("Round(1.23456, 2)").unwrap(), 1.23));
        assert!(close(eval("Clamp(15, 0, 10) + Max(1, 2) + Floor(2.7)").unwrap(), 14.0));
        assert!(close(eval("0x1F + 0b11").unwrap(), 34.0));
        assert!(close(eval("PI").unwrap(), std::f64::consts::PI));
    }

    #[test]
    fn names_resolve_to_measures() {
        let resolve = |n: &str| (n == "MeasureCPU").then_some(40.0);
        assert!(close(eval_with("MeasureCPU / 100", &resolve).unwrap(), 0.4));
        assert!(eval("Desconhecida + 1").is_err());
    }

    #[test]
    fn number_accepts_plain_or_formula() {
        assert_eq!(number("12.5"), Some(12.5));
        assert_eq!(number("(10*2)"), Some(20.0));
        assert_eq!(number("abc"), None);
    }
}
