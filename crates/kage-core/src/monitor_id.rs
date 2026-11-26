//! Identificação de monitor que não muda quando o Windows renomeia `\\.\DISPLAYn`.
//!
//! O Windows numera os monitores (`\\.\DISPLAY1`, `\\.\DISPLAY2`...) de novo quando o driver de
//! vídeo é instalado ou trocado: depois de uma atualização do driver eles podem virar
//! `\\.\DISPLAY6` e `\\.\DISPLAY7`, e até o caminho completo do dispositivo muda. O que não muda é
//! o modelo gravado na própria tela (EDID), como `DELA222` ou `AOC2402`, que o Windows põe no
//! caminho da interface do monitor. É por ele que a configuração guarda o wallpaper e a skin de
//! cada tela; o nome `\\.\DISPLAYn` fica só como reserva, para tela sem modelo.

use std::collections::HashMap;

use windows::Win32::Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW};
use windows_core::HSTRING;

/// `EDD_GET_DEVICE_INTERFACE_NAME`: pede o caminho da interface (`\\?\DISPLAY#DELA222#...`).
const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 0x1;

/// Uma identificação para cada monitor, na mesma ordem da lista. Entra o nome que o Windows dá
/// (`\\.\DISPLAY1`) e o canto superior esquerdo de cada um, que desempata monitores iguais.
pub fn monitor_ids(monitors: &[(String, i32, i32)]) -> Vec<String> {
    let list: Vec<(String, Option<String>, i32, i32)> = monitors
        .iter()
        .map(|(name, left, top)| {
            let model = interface_path(name).as_deref().and_then(model_from_interface_path);
            (name.clone(), model, *left, *top)
        })
        .collect();
    stable_ids(&list)
}

/// Caminho da interface do monitor ligado a `\\.\DISPLAYn`.
fn interface_path(device_name: &str) -> Option<String> {
    let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    let found = unsafe {
        EnumDisplayDevicesW(&HSTRING::from(device_name), 0, &mut device, EDD_GET_DEVICE_INTERFACE_NAME)
    };
    if !found.as_bool() {
        return None;
    }
    let len = device.DeviceID.iter().position(|&c| c == 0).unwrap_or(device.DeviceID.len());
    Some(String::from_utf16_lossy(&device.DeviceID[..len]))
}

/// `DELA222` de `\\?\DISPLAY#DELA222#5&379ae4ef&0&UID24835#{e6f07b5f-...}`. Tela sem EDID
/// (`Default_Monitor`) ou caminho em outro formato não tem modelo.
pub fn model_from_interface_path(path: &str) -> Option<String> {
    let mut parts = path.split('#');
    if !parts.next()?.to_ascii_uppercase().ends_with("DISPLAY") {
        return None;
    }
    let model = parts.next()?.trim();
    let valid = !model.is_empty() && model.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then(|| model.to_ascii_uppercase())
}

/// O modelo de cada monitor; com dois monitores do mesmo modelo, o primeiro da esquerda para a
/// direita (e de cima para baixo) fica só com o modelo e os outros ganham `#2`, `#3`. Sem modelo,
/// fica o nome do Windows. Entra (nome, modelo, esquerda, topo).
pub fn stable_ids(monitors: &[(String, Option<String>, i32, i32)]) -> Vec<String> {
    let mut order: Vec<usize> = (0..monitors.len()).collect();
    order.sort_by_key(|&i| (monitors[i].2, monitors[i].3));
    let mut ids = vec![String::new(); monitors.len()];
    let mut seen: HashMap<String, usize> = HashMap::new();
    for i in order {
        let (name, model, _, _) = &monitors[i];
        ids[i] = match model {
            Some(model) => {
                let count = seen.entry(model.clone()).or_insert(0);
                *count += 1;
                if *count == 1 { model.clone() } else { format!("{model}#{count}") }
            }
            None => name.clone(),
        };
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_comes_from_the_interface_path() {
        let path = r"\\?\DISPLAY#DELA222#5&379ae4ef&0&UID24835#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        assert_eq!(model_from_interface_path(path).as_deref(), Some("DELA222"));
        assert_eq!(model_from_interface_path(r"\\?\display#aoc2402#5&1#{x}").as_deref(), Some("AOC2402"));
        assert_eq!(model_from_interface_path(r"\\?\DISPLAY#Default_Monitor#4&1#{x}"), None, "tela sem EDID");
        assert_eq!(model_from_interface_path(r"\\.\DISPLAY1"), None);
        assert_eq!(model_from_interface_path(""), None);
    }

    #[test]
    fn ids_survive_a_rename_and_tell_twin_monitors_apart() {
        let before = stable_ids(&[
            (r"\\.\DISPLAY1".into(), Some("DELA222".into()), 0, 0),
            (r"\\.\DISPLAY2".into(), Some("AOC2402".into()), -1920, 0),
        ]);
        // Depois da troca de driver: nomes novos, ordem do Windows trocada.
        let after = stable_ids(&[
            (r"\\.\DISPLAY7".into(), Some("AOC2402".into()), -1920, 0),
            (r"\\.\DISPLAY6".into(), Some("DELA222".into()), 0, 0),
        ]);
        assert_eq!(before, ["DELA222", "AOC2402"]);
        assert_eq!(after, ["AOC2402", "DELA222"]);

        let twins = stable_ids(&[
            (r"\\.\DISPLAY2".into(), Some("DELA222".into()), 2560, 0),
            (r"\\.\DISPLAY1".into(), Some("DELA222".into()), 0, 0),
            (r"\\.\DISPLAY3".into(), None, 5120, 0),
        ]);
        assert_eq!(twins, ["DELA222#2", "DELA222", r"\\.\DISPLAY3"]);
    }
}
