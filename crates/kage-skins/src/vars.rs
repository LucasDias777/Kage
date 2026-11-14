//! Variáveis da skin: `#Nome#` troca pelo valor, sem diferença de maiúsculas.

use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Variables {
    map: HashMap<String, String>,
}

/// Limite para variáveis que citam outras variáveis (e para ciclos acidentais).
const MAX_DEPTH: usize = 8;

impl Variables {
    pub fn set(&mut self, name: &str, value: &str) {
        self.map.insert(name.to_lowercase(), value.to_string());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.map.get(&name.to_lowercase()).map(String::as_str)
    }

    /// Troca as variáveis conhecidas; `#Desconhecida#` e `%#d` (formato de data) ficam como estão.
    /// `#*Nome*#` é a forma escapada e vira o texto literal `#Nome#`.
    pub fn substitute(&self, text: &str) -> String {
        self.substitute_at(text, 0)
    }

    fn substitute_at(&self, text: &str, depth: usize) -> String {
        if depth > MAX_DEPTH || !text.contains('#') {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find('#') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find('#') else {
                out.push_str(&rest[start..]);
                return out;
            };
            let name = &after[..end];
            if name.len() >= 2 && name.starts_with('*') && name.ends_with('*') {
                out.push('#');
                out.push_str(&name[1..name.len() - 1]);
                out.push('#');
                rest = &after[end + 1..];
            } else if let Some(value) = (!name.is_empty()).then(|| self.get(name)).flatten() {
                out.push_str(&self.substitute_at(value, depth + 1));
                rest = &after[end + 1..];
            } else {
                // Não é variável: mantém o '#' e continua procurando a partir do próximo.
                out.push('#');
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Variables {
        let mut v = Variables::default();
        v.set("@", "C:\\Skins\\Mond\\@Resources\\");
        v.set("Language", "Português");
        v.set("Format", "H");
        v.set("Scale", "0.86");
        v.set("Double", "(#Scale#*2)");
        v
    }

    #[test]
    fn replaces_known_variables_case_insensitive() {
        assert_eq!(
            vars().substitute("#@#Language\\#language#.inc"),
            "C:\\Skins\\Mond\\@Resources\\Language\\Português.inc"
        );
        assert_eq!(vars().substitute("(14*#Scale#)"), "(14*0.86)");
    }

    #[test]
    fn keeps_date_flags_and_unknown_names() {
        assert_eq!(vars().substitute("%#Format#:%M"), "%H:%M");
        assert_eq!(vars().substitute("%#d de %B"), "%#d de %B");
        assert_eq!(vars().substitute("%#H:%#M"), "%#H:%#M");
        assert_eq!(vars().substitute("#Nada#"), "#Nada#");
    }

    #[test]
    fn nested_and_escaped() {
        assert_eq!(vars().substitute("#Double#"), "(0.86*2)");
        assert_eq!(vars().substitute("#*Scale*#"), "#Scale#");
    }

    #[test]
    fn self_reference_does_not_loop_forever() {
        let mut v = Variables::default();
        v.set("A", "x#A#");
        assert!(v.substitute("#A#").starts_with("xxxx"));
    }
}
