//! Host-selected editor presentation, independent of project state and storage policy.

/// Refusal for the five upstream community commands and their exact URLs.
pub const COMMUNITY_REFUSAL: &str = "Community navigation is unavailable in the LTS editor";
/// Refusal for every other URL a host-embedded editor is asked to open.
pub const EXTERNAL_LINK_REFUSAL: &str = "External links are unavailable in the LTS editor";
/// Label of a command this presentation hides; its saved shortcut keeps its key.
pub const UNAVAILABLE_COMMAND: &str = "Unavailable in this editor";

/// Selected by the host before the first frame, never read from a project or preference file.
/// This is presentation data; it grants no command, storage or asset authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Presentation {
    #[default]
    Standalone,
    LtsEmbedded,
}

impl Presentation {
    pub fn shows_community(self) -> bool {
        self == Self::Standalone
    }

    /// Whether the editor may open a URL outside itself. A host-embedded editor never does: a new
    /// tab is not its to open, and leaving its own tab would discard the host's session.
    pub fn allows_external_navigation(self) -> bool {
        self == Self::Standalone
    }

    pub fn allows_command(self, id: &str) -> bool {
        self.shows_community() || crate::links::url_for(id).is_none()
    }

    /// The editor's name in window titles, headings and failure messages.
    pub fn editor_name(self) -> &'static str {
        match self {
            Self::Standalone => "FilmCraft",
            Self::LtsEmbedded => "LTS Video editor",
        }
    }

    pub fn command_label<'a>(self, id: &str, original: &'a str) -> &'a str {
        if self == Self::LtsEmbedded {
            match id {
                "help.filmcraftHelp" => "Editing shortcuts",
                "app.about" => "Editing engine information",
                "file.exportSelectionProject" => "Selection as Project…",
                "app.quit" => "Quit editor",
                _ => original,
            }
        } else {
            original
        }
    }

    /// The label shortcut listings carry (`shortcuts.list`, `shortcuts.get`): a hidden command
    /// reads as unavailable, every other one as its [`Self::command_label`].
    pub fn listed_label<'a>(self, id: &str, original: &'a str) -> &'a str {
        if self.allows_command(id) { self.command_label(id, original) } else { UNAVAILABLE_COMMAND }
    }

    /// How a keyboard-shortcut preset is shown. The stored name, which `shortcuts.loadPreset` and
    /// the `shortcuts.preset.<name>` automation id use, never changes.
    pub fn preset_label(self, stored: &str) -> &str {
        if self == Self::LtsEmbedded && stored == filmcraft_engine::shortcuts::DEFAULT_PRESET { "Default" } else { stored }
    }

    /// How a Project Settings renderer value is shown. The stored value (saved in the project) never
    /// changes.
    pub fn renderer_label(self, stored: &str) -> &str {
        if self == Self::LtsEmbedded { stored.strip_prefix("FilmCraft ").unwrap_or(stored) } else { stored }
    }
}
