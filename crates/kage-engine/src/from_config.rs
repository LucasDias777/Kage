//! Converte o `config.json` (escrito pela janela do Kage) nos argumentos de uma cena do motor.

use std::path::PathBuf;
use std::time::Duration;

use kage_core::Config;

use crate::monitors::Monitor;
use crate::pause::{AppAction, Rule, Rules, Scope};
use crate::{Args, Arrangement, FitMode, SkinSource};

/// `None` quando não há nada para tocar (nenhum wallpaper escolhido ou arquivo sumiu).
pub fn args(config: &Config, monitors: &[Monitor]) -> Option<Args> {
    let path_of = |id: Option<&str>| -> Option<PathBuf> {
        let wallpaper = config.wallpaper(id?)?;
        wallpaper.path.is_file().then(|| wallpaper.path.clone())
    };
    let arrangement = match config.arrangement {
        kage_core::Arrangement::Duplicate => Arrangement::Duplicate(path_of(config.shared.as_deref())?),
        kage_core::Arrangement::Span => Arrangement::Span(path_of(config.shared.as_deref())?),
        kage_core::Arrangement::PerScreen => {
            // Pela identificação estável; o nome do Windows fica de reserva para config antigo.
            let list: Vec<(usize, PathBuf)> = monitors
                .iter()
                .filter_map(|m| {
                    let id = config.screen_wallpaper(&m.id).or_else(|| config.screen_wallpaper(&m.device_name));
                    Some((m.number, path_of(id)?))
                })
                .collect();
            if list.is_empty() {
                return None;
            }
            Arrangement::PerScreen(list)
        }
    };

    let p = &config.performance;
    let rule = |r: kage_core::Rule| if r == kage_core::Rule::Pause { Rule::Pause } else { Rule::Continue };
    let rules = Rules {
        fullscreen: rule(p.fullscreen),
        focus: rule(p.focus),
        battery: rule(p.battery),
        battery_saver: rule(p.battery_saver),
        remote_desktop: rule(p.remote_desktop),
        scope: if p.scope == kage_core::Scope::All { Scope::All } else { Scope::PerScreen },
        apps: p
            .apps
            .iter()
            .map(|a| {
                let action = match a.action {
                    kage_core::AppAction::Pause => AppAction::Pause,
                    kage_core::AppAction::Ignore => AppAction::Ignore,
                    kage_core::AppAction::Release => AppAction::Release,
                };
                (a.exe.trim().to_ascii_lowercase(), action)
            })
            .collect(),
    };

    let files = crate::arrangement_files(&arrangement);
    let looks = config
        .library
        .iter()
        .filter(|w| files.contains(&w.path.as_path()) && w.look != kage_core::Look::default())
        .map(|w| (w.path.clone(), w.look))
        .collect();

    Some(Args {
        looks,
        arrangement,
        mode: match config.fit {
            kage_core::Fit::Fill => FitMode::Fill,
            kage_core::Fit::Fit => FitMode::Fit,
            kage_core::Fit::Stretch => FitMode::Stretch,
        },
        seconds: None,
        rules,
        release_after: p.release_after_secs.map(Duration::from_secs),
        // As skins do config.json ficam com o SkinHost, fora da cena do wallpaper.
        skins: SkinSource::None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::RECT;

    fn monitors() -> Vec<Monitor> {
        vec![
            Monitor { number: 1, device_name: r"\\.\DISPLAY1".into(), id: "DELA222".into(), rect: RECT { left: 0, top: 0, right: 2560, bottom: 1440 }, primary: true },
            Monitor { number: 2, device_name: r"\\.\DISPLAY2".into(), id: "AOC2402".into(), rect: RECT { left: -1920, top: 0, right: 0, bottom: 1080 }, primary: false },
        ]
    }

    #[test]
    fn wallpaper_follows_the_screen_after_windows_renames_it() {
        let mut config = Config::default();
        let a = config.add_file(&real_file("c.mp4")).unwrap().id.clone();
        config.assign("AOC2402", Some(&a));
        // Depois de uma troca de driver o Windows chama o mesmo monitor de \\.\DISPLAY7.
        let mut renamed = monitors();
        renamed[1].device_name = r"\\.\DISPLAY7".into();
        let scene = args(&config, &renamed).unwrap();
        assert_eq!(scene.arrangement, Arrangement::PerScreen(vec![(2, real_file("c.mp4"))]));
    }

    /// Um arquivo de verdade, porque a cena só usa wallpaper que existe no disco.
    fn real_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kage-from-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn nothing_chosen_means_no_scene() {
        assert!(args(&Config::default(), &monitors()).is_none());
    }

    #[test]
    fn per_screen_uses_device_names_and_skips_missing_files() {
        let mut config = Config::default();
        let a = config.add_file(&real_file("a.mp4")).unwrap().id.clone();
        let gone = config.add_file(std::path::Path::new(r"C:\nao\existe.mp4")).unwrap().id.clone();
        config.assign(r"\\.\DISPLAY2", Some(&a));
        config.assign(r"\\.\DISPLAY1", Some(&gone));
        let scene = args(&config, &monitors()).unwrap();
        assert_eq!(scene.arrangement, Arrangement::PerScreen(vec![(2, real_file("a.mp4"))]));
        assert_eq!(scene.release_after, Some(Duration::from_secs(120)));
        assert_eq!(scene.skins, SkinSource::None);
    }

    #[test]
    fn span_rules_and_looks_come_through_and_skins_stay_out_of_the_scene() {
        let mut config = Config::default();
        let a = config.add_file(&real_file("b.gif")).unwrap().id.clone();
        config.library[0].look.speed = 150.0;
        config.arrangement = kage_core::Arrangement::Span;
        config.shared = Some(a);
        config.fit = kage_core::Fit::Fit;
        config.performance.focus = kage_core::Rule::Pause;
        config.performance.release_after_secs = None;
        config.performance.apps.push(kage_core::AppRule { exe: "OBS64.exe".into(), action: kage_core::AppAction::Release });
        config.skins.enabled = true;
        config.skins.items.push(kage_core::SkinEntry { config: "Mond\\Clock".into(), active: true, ..Default::default() });

        let scene = args(&config, &monitors()).unwrap();
        assert_eq!(scene.arrangement, Arrangement::Span(real_file("b.gif")));
        assert_eq!(scene.mode, FitMode::Fit);
        assert_eq!(scene.rules.focus, Rule::Pause);
        assert_eq!(scene.rules.apps, vec![("obs64.exe".to_string(), AppAction::Release)]);
        assert_eq!(scene.release_after, None);
        assert_eq!(scene.looks.len(), 1);
        assert_eq!(scene.looks[0].1.speed, 150.0);
        assert_eq!(scene.skins, SkinSource::None, "mudar skin não refaz a cena do wallpaper");

        // Mudar só as skins dá a mesma cena: o vídeo não reinicia.
        config.skins.items[0].x = 500.0;
        assert_eq!(args(&config, &monitors()).unwrap(), scene);
    }
}
