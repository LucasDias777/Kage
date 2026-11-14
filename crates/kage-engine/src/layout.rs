//! Onde a mídia aparece em cada monitor: que pedaço dela e em que retângulo.
//!
//! O encaixe é calculado num "canvas" em coordenadas de tela. Em Tela e Duplicar o canvas é o
//! próprio monitor; em Span é a tela virtual inteira e cada monitor recorta o pedaço dele, então a
//! emenda entre monitores fica contínua.

use windows::Win32::Foundation::RECT;

use crate::FitMode;

/// Retângulo normalizado (0 a 1) dentro da mídia.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl NormRect {
    pub const FULL: Self = Self { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 };
}

/// Pedaço da mídia (`source`) e o retângulo onde ele é desenhado (`target`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub source: NormRect,
    pub target: RECT,
}

/// Encaixa a mídia no canvas. O destino sai nas mesmas coordenadas do canvas.
pub fn place(media: (u32, u32), canvas: RECT, mode: FitMode) -> Placement {
    let canvas_w = (canvas.right - canvas.left) as f32;
    let canvas_h = (canvas.bottom - canvas.top) as f32;
    let media_aspect = media.0 as f32 / media.1.max(1) as f32;
    let canvas_aspect = canvas_w / canvas_h.max(1.0);

    match mode {
        FitMode::Stretch => Placement { source: NormRect::FULL, target: canvas },
        // Preencher: corta as sobras da mídia para cobrir o canvas inteiro.
        FitMode::Fill => {
            let mut source = NormRect::FULL;
            if media_aspect > canvas_aspect {
                let margin = (1.0 - canvas_aspect / media_aspect) / 2.0;
                source.left = margin;
                source.right = 1.0 - margin;
            } else {
                let margin = (1.0 - media_aspect / canvas_aspect) / 2.0;
                source.top = margin;
                source.bottom = 1.0 - margin;
            }
            Placement { source, target: canvas }
        }
        // Ajustar: mídia inteira visível, com faixas pretas.
        FitMode::Fit => {
            let mut target = canvas;
            if media_aspect > canvas_aspect {
                let height = (canvas_w / media_aspect).round() as i32;
                target.top = canvas.top + ((canvas_h as i32 - height) / 2);
                target.bottom = target.top + height;
            } else {
                let width = (canvas_h * media_aspect).round() as i32;
                target.left = canvas.left + ((canvas_w as i32 - width) / 2);
                target.right = target.left + width;
            }
            Placement { source: NormRect::FULL, target }
        }
    }
}

/// Recorta o encaixe para um monitor. O destino volta em coordenadas locais do monitor
/// (0,0 no canto dele). `None` quando a mídia não chega nesse monitor.
pub fn clip(placement: &Placement, monitor: RECT) -> Option<Placement> {
    let t = placement.target;
    let left = t.left.max(monitor.left);
    let top = t.top.max(monitor.top);
    let right = t.right.min(monitor.right);
    let bottom = t.bottom.min(monitor.bottom);
    if left >= right || top >= bottom {
        return None;
    }

    let target_w = (t.right - t.left) as f32;
    let target_h = (t.bottom - t.top) as f32;
    let s = placement.source;
    let (source_w, source_h) = (s.right - s.left, s.bottom - s.top);
    let source = NormRect {
        left: s.left + (left - t.left) as f32 / target_w * source_w,
        right: s.left + (right - t.left) as f32 / target_w * source_w,
        top: s.top + (top - t.top) as f32 / target_h * source_h,
        bottom: s.top + (bottom - t.top) as f32 / target_h * source_h,
    };
    Some(Placement {
        source,
        target: RECT {
            left: left - monitor.left,
            top: top - monitor.top,
            right: right - monitor.left,
            bottom: bottom - monitor.top,
        },
    })
}

/// Menor retângulo que contém todos os monitores (a tela virtual do Span).
pub fn bounding(rects: impl IntoIterator<Item = RECT>) -> RECT {
    rects.into_iter().fold(
        RECT { left: i32::MAX, top: i32::MAX, right: i32::MIN, bottom: i32::MIN },
        |acc, r| RECT {
            left: acc.left.min(r.left),
            top: acc.top.min(r.top),
            right: acc.right.max(r.right),
            bottom: acc.bottom.max(r.bottom),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    const EPS: f32 = 1e-4;

    #[test]
    fn fill_crops_wide_media_on_narrower_screen() {
        let p = place((4096, 2304), rect(0, 0, 1920, 1200), FitMode::Fill);
        assert!(p.source.left > 0.0 && p.source.right < 1.0);
        assert_eq!((p.source.top, p.source.bottom), (0.0, 1.0));
        assert_eq!(p.target, rect(0, 0, 1920, 1200));
    }

    #[test]
    fn fit_letterboxes_wide_media() {
        let p = place((3840, 1600), rect(0, 0, 2560, 1440), FitMode::Fit);
        assert_eq!(p.source, NormRect::FULL);
        assert!(p.target.top > 0 && p.target.bottom < 1440);
        assert_eq!(p.target.right - p.target.left, 2560);
    }

    #[test]
    fn same_aspect_fills_without_crop() {
        let p = place((3840, 2160), rect(0, 0, 2560, 1440), FitMode::Fill);
        assert!(p.source.left.abs() < EPS && (p.source.right - 1.0).abs() < EPS);
    }

    #[test]
    fn clip_on_own_monitor_returns_local_target() {
        let monitor = rect(-1920, 0, 0, 1080);
        let p = place((3840, 2160), monitor, FitMode::Fill);
        let local = clip(&p, monitor).unwrap();
        assert_eq!(local.target, rect(0, 0, 1920, 1080));
        assert_eq!(local.source, p.source);
    }

    #[test]
    fn span_is_continuous_across_the_seam() {
        // Dois monitores: 1080p à esquerda e 1440p como principal.
        let left = rect(-1920, 0, 0, 1080);
        let main = rect(0, 0, 2560, 1440);
        let canvas = bounding([left, main]);
        assert_eq!(canvas, rect(-1920, 0, 2560, 1440));

        let p = place((3840, 2160), canvas, FitMode::Fill);
        let a = clip(&p, left).unwrap();
        let b = clip(&p, main).unwrap();
        // O pedaço da esquerda termina exatamente onde o da direita começa.
        assert!((a.source.right - b.source.left).abs() < EPS);
        // A mídia toda é usada na horizontal, de ponta a ponta.
        assert!(a.source.left.abs() < EPS && (b.source.right - 1.0).abs() < EPS);
        // O monitor menor mostra só a faixa de cima do canvas.
        assert!(a.source.bottom < b.source.bottom);
    }

    #[test]
    fn clip_outside_media_is_none() {
        let p = Placement { source: NormRect::FULL, target: rect(0, 0, 100, 100) };
        assert!(clip(&p, rect(200, 0, 300, 100)).is_none());
    }
}
