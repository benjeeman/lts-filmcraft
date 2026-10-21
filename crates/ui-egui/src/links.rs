//! The upstream community and project links of the standalone app (Help menu, About dialog,
//! header Discord button, Home screen), and [`open`], the one place the editor opens a URL. A
//! host-embedded editor ([`Presentation::LtsEmbedded`]) opens no external URL at all.

use crate::presentation::{COMMUNITY_REFUSAL, EXTERNAL_LINK_REFUSAL, Presentation};

/// The app's short name, used in the website and repository URLs.
pub const APP: &str = "filmcraft";

/// The ArtCraft community Discord.
pub const DISCORD: &str = "https://discord.gg/artcraft";
/// The ArtCraft website.
pub const WEBSITE: &str = "https://getartcraft.com";
/// FilmCraft's page on the ArtCraft website.
pub const APP_PAGE: &str = "https://getartcraft.com/apps/filmcraft";
/// FilmCraft's source repository.
pub const GITHUB: &str = "https://github.com/storytold/filmcraft";
/// New issue on FilmCraft's repository.
pub const ISSUES: &str = "https://github.com/storytold/filmcraft/issues";

/// (command id, label, url) for every link, in menu order.
pub const ALL: [(&str, &str, &str); 5] = [
    ("help.discord", "Join the ArtCraft Discord…", DISCORD),
    ("help.website", "ArtCraft Website", WEBSITE),
    ("help.appPage", "FilmCraft on getartcraft.com", APP_PAGE),
    ("help.github", "FilmCraft on GitHub", GITHUB),
    ("help.reportIssue", "Report an Issue…", ISSUES),
];

/// The URL a `help.*` link command opens.
pub fn url_for(command: &str) -> Option<&'static str> {
    ALL.iter().find(|(id, _, _)| *id == command).map(|(_, _, u)| *u)
}

/// Open `url` in the system browser (a new tab on the web). A host-embedded editor opens nothing:
/// the five community links keep their own refusal and every other URL is refused as external.
pub fn open(presentation: Presentation, ctx: &egui::Context, url: &str) -> Result<(), String> {
    if !presentation.allows_external_navigation() {
        let community = ALL.iter().any(|(_, _, community_url)| *community_url == url);
        return Err(if community { COMMUNITY_REFUSAL } else { EXTERNAL_LINK_REFUSAL }.into());
    }
    ctx.open_url(egui::OpenUrl::new_tab(url));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_the_artcraft_scheme() {
        assert_eq!(APP_PAGE, format!("{WEBSITE}/apps/{APP}"));
        assert_eq!(GITHUB, format!("https://github.com/storytold/{APP}"));
        assert!(ALL.iter().all(|(id, _, u)| id.starts_with("help.") && u.starts_with("https://")));
        assert_eq!(url_for("help.discord"), Some(DISCORD));
        assert_eq!(url_for("help.nope"), None);
    }
}
