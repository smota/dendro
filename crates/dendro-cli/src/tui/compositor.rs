//! OpenTUI-inspired RGBA Porter-Duff Alpha Compositor and Theme Palette for Ratatui buffers.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

/// RGBA color value used for Porter-Duff source-over blending directly on `ratatui::Buffer` cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Blend `self` (source) over `dst` `(r, g, b)` using Porter-Duff source-over compositing.
    pub fn blend_over(self, dst: (u8, u8, u8)) -> (u8, u8, u8) {
        let alpha = self.a.clamp(0.0, 1.0);
        let inv = 1.0 - alpha;
        let r = (self.r as f32 * alpha + dst.0 as f32 * inv).round() as u8;
        let g = (self.g as f32 * alpha + dst.1 as f32 * inv).round() as u8;
        let b = (self.b as f32 * alpha + dst.2 as f32 * inv).round() as u8;
        (r, g, b)
    }
}

fn color_to_rgb(color: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (9, 13, 22),
        Color::DarkGray => (71, 85, 105),
        Color::Gray => (148, 163, 184),
        Color::White => (226, 232, 240),
        Color::Green | Color::LightGreen => (16, 185, 129),
        Color::Yellow | Color::LightYellow => (245, 158, 11),
        Color::Red | Color::LightRed => (239, 68, 68),
        Color::Magenta | Color::LightMagenta => (236, 72, 153),
        Color::Blue | Color::LightBlue => (139, 92, 246),
        Color::Cyan | Color::LightCyan => (6, 182, 212),
        _ => default,
    }
}

/// Applies Porter-Duff RGBA source-over alpha blending across `area` in `buf`,
/// dimming both foreground and background colors of underlying cells so modal overlays
/// float over a translucent glass backdrop rather than wiping the screen to black.
pub fn apply_rgba_backdrop(buf: &mut Buffer, area: Rect, overlay: Rgba, no_color: bool) {
    if no_color {
        return;
    }
    let bounds = area.intersection(buf.area);
    for y in bounds.top()..bounds.bottom() {
        for x in bounds.left()..bounds.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                let fg_rgb = color_to_rgb(cell.fg, (203, 213, 225));
                let bg_rgb = color_to_rgb(cell.bg, (9, 13, 22));
                let (fr, fg, fb) = overlay.blend_over(fg_rgb);
                let (br, bg, bb) = overlay.blend_over(bg_rgb);
                cell.set_fg(Color::Rgb(fr, fg, fb));
                cell.set_bg(Color::Rgb(br, bg, bb));
            }
        }
    }
}

/// Redacts secrets (`dendro_core::redact`) and scrubs/escapes all C0 and C1 control characters
/// so untrusted session logs or seat labels can never inject terminal escape sequences.
pub fn sanitize_tui_str(raw: &str) -> String {
    let redacted = dendro_core::redact(raw);
    let mut out = String::with_capacity(redacted.len());
    for ch in redacted.chars() {
        if ch.is_control() {
            use std::fmt::Write as _;
            let _ = write!(out, "\\u{:04x}", ch as u32);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Semantic theme supporting TrueColor/RGB and `NO_COLOR` accessible glyph-only fallback.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub no_color: bool,
}

impl Theme {
    pub fn detect() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some()
            || std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false);
        Self { no_color }
    }

    pub fn style(&self, fg: Color, bg: Option<Color>, modifier: Modifier) -> Style {
        if self.no_color {
            return Style::default().add_modifier(modifier);
        }
        let mut s = Style::default().fg(fg).add_modifier(modifier);
        if let Some(b) = bg {
            s = s.bg(b);
        }
        s
    }

    pub fn bg_base(&self) -> Style {
        if self.no_color {
            Style::default()
        } else {
            Style::default()
                .fg(Color::Rgb(226, 232, 240))
                .bg(Color::Rgb(9, 13, 22))
        }
    }

    pub fn border(&self, focused: bool) -> Style {
        if self.no_color {
            Style::default()
        } else if focused {
            Style::default().fg(Color::Rgb(6, 182, 212))
        } else {
            Style::default().fg(Color::Rgb(37, 54, 76))
        }
    }

    pub fn band_running(&self) -> Style {
        self.style(Color::Rgb(16, 185, 129), None, Modifier::BOLD)
    }

    pub fn band_idle_with_work(&self) -> Style {
        self.style(Color::Rgb(245, 158, 11), None, Modifier::BOLD)
    }

    pub fn band_container_hold(&self) -> Style {
        self.style(Color::Rgb(148, 163, 184), None, Modifier::DIM)
    }

    pub fn band_idle_no_work(&self) -> Style {
        self.style(Color::Rgb(100, 116, 139), None, Modifier::empty())
    }

    pub fn band_needs_input(&self) -> Style {
        self.style(Color::Rgb(236, 72, 153), None, Modifier::BOLD)
    }

    pub fn band_unknown(&self) -> Style {
        self.style(Color::Rgb(139, 92, 246), None, Modifier::DIM)
    }

    pub fn breach(&self) -> Style {
        self.style(Color::Rgb(239, 68, 68), None, Modifier::BOLD)
    }

    pub fn warn(&self) -> Style {
        self.style(Color::Rgb(245, 158, 11), None, Modifier::BOLD)
    }

    pub fn accent(&self) -> Style {
        self.style(Color::Rgb(94, 234, 212), None, Modifier::BOLD)
    }

    pub fn muted(&self) -> Style {
        self.style(Color::Rgb(148, 163, 184), None, Modifier::DIM)
    }

    pub fn unavailable(&self) -> Style {
        self.style(Color::Rgb(71, 85, 105), None, Modifier::ITALIC)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_porter_duff_blend_and_backdrop_work() {
        let overlay = Rgba::new(10, 20, 30, 0.5);
        let blended = overlay.blend_over((100, 120, 140));
        assert_eq!(blended, (55, 70, 85));

        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        buf.cell_mut((1, 1))
            .unwrap()
            .set_fg(Color::Rgb(200, 200, 200))
            .set_bg(Color::Rgb(100, 100, 100));
        apply_rgba_backdrop(&mut buf, area, Rgba::new(0, 0, 0, 0.5), false);
        assert_eq!(buf.cell((1, 1)).unwrap().fg, Color::Rgb(100, 100, 100));
        assert_eq!(buf.cell((1, 1)).unwrap().bg, Color::Rgb(50, 50, 50));
    }

    #[test]
    fn sanitize_tui_str_redacts_secrets_and_escapes_c0_c1_controls() {
        let dirty = "seat-sk-1234567890abcdefghijklmnop\u{001b}[2J\u{009b}31m\u{0085}";
        let clean = sanitize_tui_str(dirty);
        assert!(!clean.contains("1234567890abcdefghijklmnop"));
        assert!(!clean.chars().any(|c| c.is_control()));
        assert!(clean.contains("\\u001b"));
        assert!(clean.contains("\\u009b"));
        assert!(clean.contains("\\u0085"));
    }
}
