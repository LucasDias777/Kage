//! A skin que vem com o Kage: o Relógio (`Kage\Relógio`), com a fonte própria Kage Display.
//!
//! Os arquivos vão dentro do executável, porque a atualização pelo app troca só os `.exe`. A janela
//! chama [`install`] ao abrir: fontes e licença são regravadas quando mudam; o `.ini` só é escrito
//! se ainda não existir, para não desfazer a escala que o usuário ajustou nele.

use std::path::{Path, PathBuf};

/// Pacote das skins do Kage, dentro da pasta de skins.
pub const SUITE: &str = "Kage";
/// Configuração do Relógio, como aparece no `config.json`.
pub const CLOCK_CONFIG: &str = "Kage\\Relógio";

const CLOCK_INI: &str = include_str!("../../../assets/skins/Kage/Relogio.ini");

/// Arquivos de `@Resources`, relativos à pasta do pacote.
const RESOURCES: [(&str, &[u8]); 3] = [
    ("@Resources\\Fonts\\KageDisplay.otf", include_bytes!("../../../assets/skins/Kage/KageDisplay.otf")),
    ("@Resources\\Fonts\\Quicksand.otf", include_bytes!("../../../assets/skins/Kage/Quicksand.otf")),
    ("@Resources\\Fonts\\OFL-Quicksand.txt", include_bytes!("../../../assets/skins/Kage/OFL-Quicksand.txt")),
];

/// O `.ini` do Relógio dentro da pasta de skins.
pub fn clock_ini(skins_path: &Path) -> PathBuf {
    skins_path.join(CLOCK_CONFIG).join("Relógio.ini")
}

/// Põe o Relógio na pasta de skins, ou atualiza as fontes dele. `small_screen` vale para a primeira
/// instalação: numa tela principal com menos de 1440 linhas o relógio começa na escala 1,06
/// (a 1,30 do arquivo é a de uma tela de 2560x1440).
///
/// Um arquivo que não grava (fonte em uso, pasta sem permissão) não impede os outros; o erro do
/// primeiro volta para quem chamou, e a próxima abertura tenta de novo.
pub fn install(skins_path: &Path, small_screen: bool) -> Result<(), String> {
    let root = skins_path.join(SUITE);
    let mut first_error = None;
    let mut note = |result: std::io::Result<()>, path: &Path| {
        if let Err(e) = result
            && first_error.is_none()
        {
            first_error = Some(format!("{}: {e}", path.display()));
        }
    };
    for (relative, bytes) in RESOURCES {
        let path = root.join(relative);
        if std::fs::read(&path).is_ok_and(|current| current == bytes) {
            continue;
        }
        note(write(&path, bytes), &path);
    }
    let ini = clock_ini(skins_path);
    if !ini.exists() {
        note(write(&ini, clock_text(small_screen).as_bytes()), &ini);
    }
    first_error.map_or(Ok(()), Err)
}

fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)
}

fn clock_text(small_screen: bool) -> String {
    if small_screen {
        CLOCK_INI.replace("\nEscala=1.30", "\nEscala=1.06").replace("\nEspacoDia=12.26", "\nEspacoDia=10")
    } else {
        CLOCK_INI.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DateTime, Item, NoSystem, Skin, TextMeasurer, TextStyle};

    struct Measurer;

    impl TextMeasurer for Measurer {
        fn measure_text(&self, text: &str, style: &TextStyle) -> (f32, f32) {
            (text.chars().count() as f32 * style.size_px * 0.6, style.size_px * 1.25)
        }
        fn image_size(&self, _path: &Path) -> Option<(f32, f32)> {
            None
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kage-bundled-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn variable(ini: &Path, name: &str) -> String {
        let text = std::fs::read_to_string(ini).unwrap();
        let prefix = format!("{name}=");
        text.lines().find_map(|l| l.strip_prefix(&prefix)).unwrap().to_string()
    }

    #[test]
    fn installs_clock_and_fonts() {
        let root = temp_root("instala");
        install(&root, false).unwrap();
        let fonts = root.join("Kage").join("@Resources").join("Fonts");
        for name in ["KageDisplay.otf", "Quicksand.otf", "OFL-Quicksand.txt"] {
            assert!(fonts.join(name).is_file(), "{name}");
        }
        let ini = clock_ini(&root);
        assert_eq!(variable(&ini, "Escala"), "1.30");
        assert_eq!(variable(&ini, "EspacoDia"), "12.26");

        // O motor lê o Relógio: dia em Kage Display, data e horário em Quicksand, em português.
        let mut skin = Skin::load(&ini, &root).unwrap();
        assert_eq!(skin.config, CLOCK_CONFIG);
        assert!(skin.warnings.is_empty(), "{:?}", skin.warnings);
        let now = DateTime { year: 2026, month: 3, day: 3, weekday: 2, hour: 9, minute: 5, second: 0, day_of_year: 62 };
        let frame = skin.update(&now, &Measurer, &NoSystem);
        let texts: Vec<(&str, &str)> = frame
            .items
            .iter()
            .filter_map(|i| if let Item::Text(t) = i { Some((t.text.as_str(), t.style.face.as_str())) } else { None })
            .collect();
        assert_eq!(
            texts,
            vec![
                ("09:05", "Quicksand"),
                ("|", "Kage Display"),
                ("TERÇA", "Kage Display"),
                ("03  MARÇO,  2026.", "Quicksand")
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn small_screen_starts_smaller() {
        let root = temp_root("tela-menor");
        install(&root, true).unwrap();
        let ini = clock_ini(&root);
        assert_eq!(variable(&ini, "Escala"), "1.06");
        assert_eq!(variable(&ini, "EspacoDia"), "10");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keeps_user_ini_and_restores_fonts() {
        let root = temp_root("mantem");
        install(&root, false).unwrap();
        let ini = clock_ini(&root);
        std::fs::write(&ini, "[Variables]\nEscala=2\n").unwrap();
        let font = root.join("Kage").join("@Resources").join("Fonts").join("KageDisplay.otf");
        std::fs::write(&font, b"velha").unwrap();

        install(&root, true).unwrap();
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "[Variables]\nEscala=2\n", "o .ini do usuário fica");
        assert_eq!(std::fs::read(&font).unwrap(), RESOURCES[0].1, "a fonte volta à do Kage");
        let _ = std::fs::remove_dir_all(&root);
    }
}
