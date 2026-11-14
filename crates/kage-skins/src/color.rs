//! Cores do Rainmeter: `255,255,255`, `255,255,255,128` ou hexadecimal `FFFFFF` / `FFFFFF80`.

use crate::formula;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Self = Self { r: 255, g: 255, b: 255, a: 255 };
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0, a: 255 };

    pub fn parse(value: &str) -> Option<Self> {
        let v = value.trim();
        if v.contains(',') {
            let parts: Vec<u8> = v
                .split(',')
                .map(|p| formula::number(p).map(|n| n.round().clamp(0.0, 255.0) as u8))
                .collect::<Option<_>>()?;
            return match parts.as_slice() {
                [r, g, b] => Some(Self { r: *r, g: *g, b: *b, a: 255 }),
                [r, g, b, a] => Some(Self { r: *r, g: *g, b: *b, a: *a }),
                _ => None,
            };
        }
        let hex = |i: usize| u8::from_str_radix(v.get(i..i + 2)?, 16).ok();
        match v.len() {
            6 => Some(Self { r: hex(0)?, g: hex(2)?, b: hex(4)?, a: 255 }),
            8 => Some(Self { r: hex(0)?, g: hex(2)?, b: hex(4)?, a: hex(6)? }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_hex_and_formulas() {
        assert_eq!(Color::parse("255,255,255"), Some(Color::WHITE));
        assert_eq!(Color::parse("250, 126, 0, 128"), Some(Color { r: 250, g: 126, b: 0, a: 128 }));
        assert_eq!(Color::parse("FA7E00"), Some(Color { r: 250, g: 126, b: 0, a: 255 }));
        assert_eq!(Color::parse("FA7E0080"), Some(Color { r: 250, g: 126, b: 0, a: 128 }));
        assert_eq!(Color::parse("(200+55),0,0"), Some(Color { r: 255, g: 0, b: 0, a: 255 }));
        assert_eq!(Color::parse("azul"), None);
    }
}
