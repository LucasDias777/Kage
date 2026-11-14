//! O `Rainmeter.ini` do usuário: pasta das skins, quais estão ativas e onde ficam na tela.

use std::path::{Path, PathBuf};

use crate::ini;

/// Opções da janela da skin, gravadas pelo Rainmeter na seção dela.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowOptions {
    /// `AlphaValue`, de 1 a 255.
    pub alpha: u8,
    /// `AlwaysOnTop`: -2 no desktop, -1 abaixo, 0 normal, 1 acima, 2 sempre visível.
    pub always_on_top: i32,
    pub draggable: bool,
    pub click_through: bool,
    pub keep_on_screen: bool,
    pub snap_edges: bool,
    pub save_position: bool,
    /// `OnHover` (antes `HideOnMouseOver`): 0 nada, 1 esconder, 2 aparecer, 3 desaparecer.
    pub on_hover: i32,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            alpha: 255,
            always_on_top: 0,
            draggable: true,
            click_through: false,
            keep_on_screen: true,
            snap_edges: true,
            save_position: true,
            on_hover: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveSkin {
    /// `Mond\Clock`.
    pub config: String,
    pub ini: PathBuf,
    /// Canto da janela da skin na tela virtual (0,0 = canto do monitor principal).
    pub x: f32,
    pub y: f32,
    pub options: WindowOptions,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub skins_path: PathBuf,
    pub active: Vec<ActiveSkin>,
}

/// `%APPDATA%\Rainmeter\Rainmeter.ini`.
pub fn default_ini() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Rainmeter").join("Rainmeter.ini"))
}

pub fn read(rainmeter_ini: &Path) -> Result<Setup, String> {
    let bytes = std::fs::read(rainmeter_ini).map_err(|e| format!("não abriu {}: {e}", rainmeter_ini.display()))?;
    let sections = ini::parse(&ini::decode(&bytes));
    let skins_path = sections
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("Rainmeter"))
        .and_then(|s| s.get("SkinPath"))
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(std::env::var_os("USERPROFILE")?).join("Documents").join("Rainmeter").join("Skins")))
        .ok_or("pasta das skins não encontrada")?;

    let mut active = Vec::new();
    for section in &sections {
        let Some(index) = section.get("Active").and_then(|a| a.trim().parse::<usize>().ok()).filter(|&i| i > 0) else {
            continue;
        };
        let folder = skins_path.join(&section.name);
        // Active=N é o N-ésimo .ini da pasta, em ordem alfabética.
        let Some(ini) = nth_ini(&folder, index) else { continue };
        let coord = |key: &str| section.get(key).map(window_coord).unwrap_or(0.0);
        let number = |key: &str| section.get(key).and_then(|v| v.trim().parse::<i32>().ok());
        let flag = |key: &str, default: bool| number(key).map_or(default, |v| v != 0);
        let defaults = WindowOptions::default();
        let options = WindowOptions {
            alpha: number("AlphaValue").map_or(255, |a| a.clamp(1, 255) as u8),
            always_on_top: number("AlwaysOnTop").unwrap_or(0).clamp(-2, 2),
            draggable: flag("Draggable", defaults.draggable),
            click_through: flag("ClickThrough", defaults.click_through),
            keep_on_screen: flag("KeepOnScreen", defaults.keep_on_screen),
            snap_edges: flag("SnapEdges", defaults.snap_edges),
            save_position: flag("SavePosition", defaults.save_position),
            on_hover: number("OnHover").or_else(|| number("HideOnMouseOver")).unwrap_or(0).clamp(0, 3),
        };
        active.push(ActiveSkin { config: section.name.clone(), ini, x: coord("WindowX"), y: coord("WindowY"), options });
    }
    Ok(Setup { skins_path, active })
}

/// Variantes de uma skin: os `.ini` da pasta dela, na ordem do Rainmeter.
pub fn variants(ini: &Path) -> Vec<PathBuf> {
    let Some(folder) = ini.parent() else { return Vec::new() };
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ini")))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_uppercase()));
    files
}

/// Todas as configurações instaladas: cada pasta com `.ini` dentro da pasta de skins, com o
/// primeiro `.ini` dela. Pastas que começam com `@` (`@Resources`, `@Vault`) ficam de fora.
pub fn installed(skins_path: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    walk(skins_path, skins_path, &mut found, 0);
    found.sort_by_key(|(config, _)| config.to_uppercase());
    found
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<(String, PathBuf)>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let (mut inis, mut folders) = (Vec::new(), Vec::new());
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            if !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('@')) {
                folders.push(path);
            }
        } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("ini")) {
            inis.push(path);
        }
    }
    if dir != root && !inis.is_empty() {
        inis.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_uppercase()));
        if let Ok(relative) = dir.strip_prefix(root) {
            found.push((relative.to_string_lossy().to_string(), inis.swap_remove(0)));
        }
    }
    for folder in folders {
        walk(root, &folder, found, depth + 1);
    }
}

fn nth_ini(folder: &Path, index: usize) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ini")))
        .collect();
    // O Rainmeter usa a ordem do FindFirstFile; no NTFS ela compara em maiúsculas.
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_uppercase()));
    files.into_iter().nth(index - 1)
}

/// Só a parte numérica; sufixos de âncora do Rainmeter (`R`, `B`, `%`, `@N`) ainda não entram.
fn window_coord(value: &str) -> f32 {
    let numeric: String = value
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
        .collect();
    numeric.parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_active_skins_and_positions() {
        let dir = std::env::temp_dir().join(format!("kage-rainmeter-{}", std::process::id()));
        let skins = dir.join("Skins");
        std::fs::create_dir_all(skins.join("Mond").join("Clock")).unwrap();
        std::fs::create_dir_all(skins.join("Mond").join("Weather")).unwrap();
        std::fs::write(skins.join("Mond").join("Clock").join("Clock.ini"), "[Rainmeter]").unwrap();
        std::fs::write(skins.join("Mond").join("Weather").join("Weather.ini"), "[Rainmeter]").unwrap();
        let ini_text = format!(
            "[Rainmeter]\nSkinPath={}\\\n[Mond\\Clock]\nActive=1\nWindowX=984\nWindowY=300\nAlphaValue=128\nAlwaysOnTop=-2\nClickThrough=1\nHideOnMouseOver=2\n\
             [Mond\\Weather]\nActive=0\n[Mond2\\Clock]\nActive=1\nWindowX=-1310\nWindowY=284\n",
            skins.display()
        );
        let mut bytes = vec![0xFF, 0xFE];
        for unit in ini_text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let path = dir.join("Rainmeter.ini");
        std::fs::write(&path, bytes).unwrap();

        let setup = read(&path).unwrap();
        // Mond2\Clock não existe na pasta: fica de fora em vez de quebrar.
        assert_eq!(setup.active.len(), 1);
        assert_eq!(setup.active[0].config, "Mond\\Clock");
        assert_eq!((setup.active[0].x, setup.active[0].y), (984.0, 300.0));
        assert!(setup.active[0].ini.ends_with("Mond\\Clock\\Clock.ini"));
        let o = &setup.active[0].options;
        assert_eq!((o.alpha, o.always_on_top, o.click_through, o.on_hover), (128, -2, true, 2));
        assert!(o.draggable && o.save_position, "o que não está no arquivo fica no padrão do Rainmeter");
        assert_eq!(variants(&setup.active[0].ini).len(), 1);
    }

    #[test]
    fn lists_installed_configs_without_resources() {
        let root = std::env::temp_dir().join(format!("kage-installed-{}", std::process::id()));
        for dir in ["Mond\\Clock", "Mond\\@Resources", "illustro\\Disk", "Vazia"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("Mond\\Clock\\Clock.ini"), "").unwrap();
        std::fs::write(root.join("Mond\\@Resources\\Variables.ini"), "").unwrap();
        std::fs::write(root.join("illustro\\Disk\\Disk 2.ini"), "").unwrap();
        std::fs::write(root.join("illustro\\Disk\\Disk 1.ini"), "").unwrap();
        let found = installed(&root);
        let names: Vec<&str> = found.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(names, vec!["illustro\\Disk", "Mond\\Clock"]);
        assert!(found[0].1.ends_with("Disk 1.ini"), "o primeiro .ini em ordem");
    }

    #[test]
    fn window_coordinates_ignore_anchor_suffixes() {
        assert_eq!(window_coord("-1310"), -1310.0);
        assert_eq!(window_coord("50%"), 50.0);
        assert_eq!(window_coord("200R"), 200.0);
    }
}
