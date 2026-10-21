//! About ▸ Third-party notices: the repository's NOTICE and licence files, the asset attribution
//! list and the licences of the fonts compiled into the editor (the bundled fonts and egui's default
//! fonts), compiled in with `include_str!` and shown as selectable plain text, never as links.
//! The list does not claim to be complete; see [`SCOPE`].

use std::sync::LazyLock;

use egui::RichText;

use crate::automation::Registry;
use crate::theme::Tokens;

/// The plain-text attribution, also shown on About ▸ About.
pub const ATTRIBUTION: &str = "Modified version based on FilmCraft by the ArtCraft team and community.";

/// What the notices cover and what they do not.
pub const SCOPE: &str = "Reproduced from the editor's source repository: its NOTICE and licence files, the asset attribution list and the licences of the fonts compiled into the editor. This is not a complete list: the licences of the Rust crates compiled into the editor are governed by Cargo.lock, and a generated dependency-licence bundle is pending.";

/// One notice: automation id suffix, title, path in the source repository and verbatim text.
pub struct Notice {
    pub id: &'static str,
    pub title: &'static str,
    pub source: &'static str,
    pub text: &'static str,
}

pub const NOTICES: &[Notice] = &[
    Notice { id: "notice", title: "NOTICE", source: "NOTICE", text: include_str!("../../../NOTICE") },
    Notice { id: "licenseMit", title: "MIT License", source: "LICENSE-MIT", text: include_str!("../../../LICENSE-MIT") },
    Notice { id: "licenseApache", title: "Apache License 2.0", source: "LICENSE-APACHE", text: include_str!("../../../LICENSE-APACHE") },
    Notice { id: "attribution", title: "Asset attribution", source: "ATTRIBUTION.md", text: include_str!("../../../ATTRIBUTION.md") },
    Notice { id: "oflInter", title: "Inter (SIL OFL 1.1)", source: "assets/fonts/OFL-Inter.txt", text: include_str!("../../../assets/fonts/OFL-Inter.txt") },
    Notice {
        id: "oflNotoSerif",
        title: "Noto Serif (SIL OFL 1.1)",
        source: "assets/fonts/OFL-NotoSerif.txt",
        text: include_str!("../../../assets/fonts/OFL-NotoSerif.txt"),
    },
    Notice {
        id: "oflJetBrainsMono",
        title: "JetBrains Mono (SIL OFL 1.1)",
        source: "assets/fonts/OFL-JetBrainsMono.txt",
        text: include_str!("../../../assets/fonts/OFL-JetBrainsMono.txt"),
    },
    Notice {
        id: "eguiDefaultFonts",
        title: "egui default fonts",
        source: "docs/notices/egui-default-fonts.txt",
        text: include_str!("../../../docs/notices/egui-default-fonts.txt"),
    },
];

/// The notice texts without carriage returns (a Windows checkout embeds them with CRLF).
static TEXTS: LazyLock<Vec<String>> = LazyLock::new(|| NOTICES.iter().map(|n| n.text.replace("\r\n", "\n")).collect());

/// Height of the scrolling notice text, so the fixed-size About window stays on screen.
const TEXT_HEIGHT: f32 = 320.0;

/// About ▸ Third-party notices: the attribution, the scope statement, one selectable title per
/// notice and the chosen notice's text. Automation ids: `about.notices.<id>` (see [`NOTICES`]).
pub fn notices_ui(ui: &mut egui::Ui, t: &Tokens, auto: &mut Registry) {
    let state = egui::Id::new("notices_selected");
    let mut selected = ui.data_mut(|d| d.get_temp::<usize>(state)).unwrap_or(0);
    ui.label(ATTRIBUTION);
    ui.label(RichText::new(SCOPE).small().color(t.text_dim));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (i, n) in NOTICES.iter().enumerate() {
            let r = ui.selectable_label(selected == i, n.title);
            auto.add(&format!("about.notices.{}", n.id), r.rect, n.title);
            if r.clicked() {
                selected = i;
            }
        }
    });
    ui.separator();
    if let (Some(n), Some(text)) = (NOTICES.get(selected), TEXTS.get(selected)) {
        ui.label(RichText::new(format!("{} ({})", n.title, n.source)).strong());
        egui::ScrollArea::vertical().id_salt(("notices_text", n.id)).max_height(TEXT_HEIGHT).auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text.as_str()).monospace().small()).selectable(true).wrap());
        });
    }
    ui.data_mut(|d| d.insert_temp(state, selected));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_are_verbatim_and_honest_about_scope() {
        assert!(NOTICES.iter().all(|n| !n.text.trim().is_empty()), "every notice has text");
        let ids: Vec<&str> = NOTICES.iter().map(|n| n.id).collect();
        assert!(ids.iter().enumerate().all(|(i, id)| !ids[..i].contains(id)), "unique ids: {ids:?}");
        assert!(NOTICES[0].text.contains("Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors."));
        assert!(SCOPE.contains("Cargo.lock") && SCOPE.contains("pending") && !SCOPE.contains("http"));
        assert!(TEXTS.iter().all(|t| !t.contains('\r')));
        let fonts = NOTICES.iter().find(|n| n.id == "eguiDefaultFonts").map(|n| n.text).unwrap_or_default();
        for licence in ["fonts/Hack-Regular.txt", "fonts/OFL.txt", "fonts/UFL.txt", "fonts/emoji-icon-font-mit-license.txt"] {
            assert!(fonts.contains(licence), "{licence}");
        }
    }
}
