//! INI do Rainmeter: seções em ordem, chaves sem diferença de maiúsculas, arquivos em UTF-16,
//! UTF-8 ou ANSI (Windows-1252).

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub name: String,
    pub keys: Vec<(String, String)>,
}

impl Section {
    /// Valor bruto da chave; se repetida, vale a última.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.keys.push((key.to_string(), value.to_string()));
    }
}

/// Texto do arquivo, conforme a marca de ordem de bytes (BOM).
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

/// Skins antigas salvas em ANSI: o Windows-1252 difere do Latin-1 só de 0x80 a 0x9F.
fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[(byte - 0x80) as usize],
        _ => byte as char,
    }
}

pub fn parse(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<Section> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            sections.extend(current.take());
            current = Some(Section { name: line[1..line.len() - 1].trim().to_string(), keys: Vec::new() });
            continue;
        }
        // O Rainmeter tira as aspas em volta já na leitura, em toda opção (ConfigParser.cpp).
        if let (Some(section), Some((key, value))) = (current.as_mut(), line.split_once('=')) {
            section.keys.push((key.trim().to_string(), unquote(value).to_string()));
        }
    }
    sections.extend(current);
    sections
}

/// Tira as aspas que envolvem o valor inteiro (`Text="- %1 -"` vira `- %1 -`).
pub fn unquote(value: &str) -> &str {
    let v = value.trim();
    if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16_with_accents() {
        let text = "[Variables]\nDate=\"Saturday\":\"Sábado\"";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode(&bytes), text);
    }

    #[test]
    fn decodes_ansi_fallback() {
        // "Março" em Windows-1252: ç = 0xE7.
        assert_eq!(decode(&[b'M', b'a', b'r', 0xE7, b'o']), "Março");
    }

    #[test]
    fn parses_sections_keys_and_comments() {
        let s = parse("; topo\n[Rainmeter]\nUpdate=1000\n\n[MeterDay]\nMeter = String\n;X=1\nText=\"%1\"\nText=\"%2\"");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].get("update"), Some("1000"));
        assert_eq!(s[1].get("METER"), Some("String"));
        assert_eq!(s[1].get("X"), None);
        assert_eq!(s[1].get("Text"), Some("%2"), "repetida vale a última, já sem aspas");
    }

    #[test]
    fn unquote_only_strips_enclosing_quotes() {
        assert_eq!(unquote("\"- %1 -\""), "- %1 -");
        assert_eq!(unquote("\"a\":\"b\""), "a\":\"b");
        assert_eq!(unquote("sem aspas"), "sem aspas");
    }
}
