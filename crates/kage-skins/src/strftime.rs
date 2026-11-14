//! Formato de data e hora do Rainmeter (o `strftime` do C da Microsoft).
//!
//! Os nomes saem em inglês, como no Rainmeter; as skins traduzem com `Substitute`.
//! O sinal `#` tira o zero à esquerda: `%#d` = "5" em vez de "05".

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    /// 1 a 12.
    pub month: u32,
    pub day: u32,
    /// 0 = domingo.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 1 a 366.
    pub day_of_year: u32,
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

pub fn format(fmt: &str, t: &DateTime) -> String {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let no_pad = chars.peek() == Some(&'#');
        if no_pad {
            chars.next();
        }
        let Some(spec) = chars.next() else {
            out.push('%');
            break;
        };
        let num = |v: u32, width: usize| if no_pad { v.to_string() } else { format!("{v:0width$}") };
        let hour12 = if t.hour.is_multiple_of(12) { 12 } else { t.hour % 12 };
        let weekday = WEEKDAYS[(t.weekday % 7) as usize];
        let month = MONTHS[((t.month.clamp(1, 12)) - 1) as usize];
        match spec {
            'a' => out.push_str(&weekday[..3]),
            'A' => out.push_str(weekday),
            'b' | 'h' => out.push_str(&month[..3]),
            'B' => out.push_str(month),
            'd' => out.push_str(&num(t.day, 2)),
            'e' => out.push_str(&format!("{:>2}", t.day)),
            'H' => out.push_str(&num(t.hour, 2)),
            'I' => out.push_str(&num(hour12, 2)),
            'j' => out.push_str(&num(t.day_of_year, 3)),
            'm' => out.push_str(&num(t.month, 2)),
            'M' => out.push_str(&num(t.minute, 2)),
            'p' => out.push_str(if t.hour < 12 { "AM" } else { "PM" }),
            'S' => out.push_str(&num(t.second, 2)),
            'w' => out.push_str(&t.weekday.to_string()),
            'y' => out.push_str(&num((t.year % 100) as u32, 2)),
            'Y' => out.push_str(&t.year.to_string()),
            'x' => out.push_str(&format!("{:02}/{:02}/{:02}", t.month, t.day, t.year % 100)),
            'X' | 'T' => out.push_str(&format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)),
            'R' => out.push_str(&format!("{:02}:{:02}", t.hour, t.minute)),
            '%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O momento do print da Mond: sexta, 13 de novembro de 2026, 20:47.
    pub(crate) fn friday_evening() -> DateTime {
        DateTime { year: 2026, month: 11, day: 13, weekday: 5, hour: 20, minute: 47, second: 5, day_of_year: 317 }
    }

    #[test]
    fn mond_formats() {
        let t = friday_evening();
        assert_eq!(format("%H:%M", &t), "20:47");
        assert_eq!(format("%A", &t), "Friday");
        assert_eq!(format("%d  %B,  %Y.", &t), "13  November,  2026.");
        assert_eq!(format("%p", &t), "PM");
    }

    #[test]
    fn hash_flag_removes_leading_zeros() {
        let t = DateTime { day: 5, hour: 7, month: 3, ..friday_evening() };
        assert_eq!(format("%d/%m %H", &t), "05/03 07");
        assert_eq!(format("%#d/%#m %#H", &t), "5/3 7");
        assert_eq!(format("%I %#I", &t), "07 7");
    }

    #[test]
    fn midnight_and_noon_in_12h() {
        let midnight = DateTime { hour: 0, ..friday_evening() };
        let noon = DateTime { hour: 12, ..friday_evening() };
        assert_eq!(format("%I %p", &midnight), "12 AM");
        assert_eq!(format("%I %p", &noon), "12 PM");
    }
}
