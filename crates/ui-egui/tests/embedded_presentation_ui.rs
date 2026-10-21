//! Public control-channel checks for standalone and host-owned editor presentation.
//! No browser, media fixture, model call or persistent project is required.

use egui::{Context, OutputCommand, RawInput};
use egui_kittest::kittest::{NodeT, Queryable};
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::{self, ControlRequest, Outcome};
use filmcraft_ui_egui::links;
use filmcraft_ui_egui::presentation::Presentation;
use serde_json::{Value, json};

fn invoke(app: &mut FilmcraftApp, ctx: &Context, method: &str, id: &str) -> (Value, Vec<String>) {
    let params = if method == "ui.menu.invoke" { json!({"id": id}) } else { json!({"command": id, "params": {}}) };
    let (request, _) = ControlRequest::new(method, params);
    let mut reply = None;
    let mut output = ctx.run_ui(RawInput::default(), |ui| {
        reply = Some(match control::handle(app, ui.ctx(), &request) {
            Outcome::Done(value) => value,
            _ => panic!("community-link invocation must return its immediate control result"),
        });
    });
    // This headless test consumes commands but deliberately does not render textures.
    output.textures_delta.clear();
    let urls = output
        .platform_output
        .commands
        .into_iter()
        .filter_map(|command| match command {
            OutputCommand::OpenUrl(url) => Some(url.url),
            _ => None,
        })
        .collect();
    (reply.expect("control handler was invoked"), urls)
}

fn assert_standalone_navigation(method: &str) {
    let mut app = FilmcraftApp::new(Session::default());
    assert_eq!(app.presentation(), Presentation::Standalone);
    let ctx = Context::default();
    for (id, _, url) in links::ALL {
        let (reply, opened) = invoke(&mut app, &ctx, method, id);
        assert_eq!(reply, json!({"ok": true, "result": {"url": url}}), "{method} {id}");
        assert_eq!(opened, vec![url.to_string()], "{method} {id} must preserve the actual standalone navigation");
    }
}

fn assert_embedded_navigation_refused(method: &str) {
    let mut app = FilmcraftApp::new(Session::default()).with_presentation(Presentation::LtsEmbedded);
    assert_eq!(app.presentation(), Presentation::LtsEmbedded);
    let ctx = Context::default();
    for (id, _, _) in links::ALL {
        let (reply, opened) = invoke(&mut app, &ctx, method, id);
        assert_eq!(
            reply,
            json!({"ok": false, "error": "Community navigation is unavailable in the LTS editor"}),
            "{method} {id} must refuse in the LTS editor"
        );
        assert!(opened.is_empty(), "{method} {id} must not enqueue browser navigation: {opened:?}");
    }
}

#[test]
fn standalone_engine_execute_community_navigation_is_preserved() {
    assert_standalone_navigation("engine.execute");
}

#[test]
fn standalone_ui_menu_invoke_community_navigation_is_preserved() {
    assert_standalone_navigation("ui.menu.invoke");
}

#[test]
fn embedded_engine_execute_community_navigation_is_refused() {
    assert_embedded_navigation_refused("engine.execute");
}

#[test]
fn embedded_ui_menu_invoke_community_navigation_is_refused() {
    assert_embedded_navigation_refused("ui.menu.invoke");
}

// Prepared additional tests to append to embedded_presentation_ui.rs after root accepts the
// dispatch RED. Not installed, compiled or executed. Existing file already supplies imports
// for Context, Session, FilmcraftApp, Presentation, ControlRequest, Value and json.

struct EmbeddedDriver {
    harness: egui_kittest::Harness<'static, FilmcraftApp>,
    tx: std::sync::mpsc::Sender<ControlRequest>,
}

impl EmbeddedDriver {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let app = FilmcraftApp::new(Session::default()).with_presentation(Presentation::LtsEmbedded).with_control(rx);
        let harness = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut driver = Self { harness, tx };
        driver.frames(4);
        driver
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).expect("control inbox stays alive");
        for _ in 0..600 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], json!(true), "{method} {params}: {value}");
                return value["result"].clone();
            }
        }
        panic!("no bounded control reply for {method} {params}");
    }

    fn elements(&mut self, prefix: &str) -> Vec<Value> {
        self.ok("ui.elements", json!({"prefix": prefix})).as_array().expect("widget array").clone()
    }

    fn ids(&mut self, prefix: &str) -> Vec<String> {
        self.elements(prefix).iter().filter_map(|value| value["id"].as_str().map(str::to_string)).collect()
    }
}

#[test]
fn embedded_surfaces_omit_community_but_keep_manual_controls_and_credits() {
    let mut d = EmbeddedDriver::new();
    let project = d.harness.state().session.project.clone();
    assert!(d.ids("header.discord").is_empty());
    assert!(!d.ids("header.workspaces").is_empty());
    assert!(!d.ids("header.mode.edit").is_empty());
    assert!(!d.ids("header.documentTitle").is_empty(), "title painting cannot disappear with Discord");

    let menu = d.ok("ui.menu.list", json!({}));
    for (hidden, _, _) in links::ALL {
        assert!(!menu.as_array().unwrap().iter().any(|row| row["id"] == hidden), "hidden menu ID {hidden}");
    }
    for kept in ["app.about", "app.keyboardShortcuts", "help.filmcraftHelp"] {
        assert!(menu.as_array().unwrap().iter().any(|row| row["id"] == kept), "useful command {kept}");
    }

    d.ok("engine.execute", json!({"command": "app.about"}));
    d.frames(3);
    let about = d.ids("about.");
    for hidden in ["about.discord", "about.website", "about.appPage", "about.github", "about.reportIssue"] {
        assert!(!about.iter().any(|id| id == hidden), "hidden About action {hidden}");
    }
    for kept in ["about.tab.about", "about.tab.contributors", "about.tab.models"] {
        assert!(about.iter().any(|id| id == kept), "attribution tab {kept}");
    }
    d.ok("ui.key", json!({"key": "Escape"}));
    d.frames(3);
    d.ok("engine.execute", json!({"command": "mode.import"}));
    d.frames(3);
    assert!(d.ids("import.link.").is_empty());
    assert!(d.ids("import.file.import").iter().any(|id| id == "import.file.import"));

    let help = d.ok("engine.execute", json!({"command": "help.filmcraftHelp"}));
    assert_eq!(help, json!({"dialog": "shortcuts"}));
    d.frames(3);
    assert!(!d.ids("shortcuts.search").is_empty());
    assert!(std::sync::Arc::ptr_eq(&project, &d.harness.state().session.project), "presentation reads must keep the same working project");
}

#[test]
fn embedded_saved_community_shortcut_is_retained_but_not_promoted() {
    let mut d = EmbeddedDriver::new();
    d.ok("engine.execute", json!({"command": "shortcuts.set", "params": {"command": "help.discord", "keys": "F12"}}));
    let original = serde_json::to_value(&d.harness.state().session.shortcuts.bindings).expect("binding fixture serializes");
    d.ok("engine.execute", json!({"command": "app.keyboardShortcuts"}));
    d.harness.state_mut().shortcut_editor.search = "Discord".into();
    d.harness.state_mut().shortcut_editor.key = Some("F12".into());
    d.frames(4);
    assert!(d.ids("shortcuts.row.help.discord").is_empty());
    assert!(d.ids("shortcuts.cell.help.discord").is_empty());
    let keyboard_key = d.elements("shortcuts.key.F12");
    assert_eq!(keyboard_key.len(), 1);
    assert_eq!(keyboard_key[0]["label"], "Unavailable in this editor");
    assert_eq!(serde_json::to_value(&d.harness.state().session.shortcuts.bindings).unwrap(), original);
}

/// Run `links::open` in one headless frame: its result and every (url, new tab) it enqueued.
fn open_link(ctx: &Context, presentation: Presentation, url: &str) -> (Result<(), String>, Vec<(String, bool)>) {
    let mut result = None;
    let mut output = ctx.run_ui(RawInput::default(), |ui| {
        result = Some(links::open(presentation, ui.ctx(), url));
    });
    output.textures_delta.clear();
    let opened = output
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            OutputCommand::OpenUrl(open) => Some((open.url.clone(), open.new_tab)),
            _ => None,
        })
        .collect();
    (result.expect("link helper ran"), opened)
}

#[test]
fn embedded_link_helper_denies_all_external_destinations() {
    let ctx = Context::default();
    for (_, _, url) in links::ALL {
        let (result, opened) = open_link(&ctx, Presentation::LtsEmbedded, url);
        assert_eq!(result, Err("Community navigation is unavailable in the LTS editor".to_string()), "{url}");
        assert!(opened.is_empty(), "{url}: {opened:?}");
    }
    let documentation = format!("{}/blob/main/docs/project-files.md#settings", links::GITHUB);
    let login = filmcraft_ui_egui::credits::CONTRIBUTORS.first().map(|c| c.login).expect("contributors are compiled in");
    let contributor = format!("https://github.com/{login}");
    for url in [filmcraft_ui_egui::panels::keyboard::HELP_URL.to_string(), documentation.clone(), contributor] {
        let (result, opened) = open_link(&ctx, Presentation::LtsEmbedded, &url);
        assert_eq!(result, Err("External links are unavailable in the LTS editor".to_string()), "{url}");
        assert!(opened.is_empty(), "{url}: {opened:?}");
    }
    // the standalone app still opens documentation, in a new tab
    let (result, opened) = open_link(&ctx, Presentation::Standalone, &documentation);
    assert_eq!(result, Ok(()));
    assert_eq!(opened, vec![(documentation, true)]);
}

#[test]
fn embedded_header_records_only_painted_dirty_title_geometry_without_control_overlap() {
    for width in [640.0, 1280.0, 1920.0] {
        let mut session = Session::default();
        session
            .edit("title fixture", |project, _| {
                project.name = "LTS title geometry".into();
                Ok(())
            })
            .unwrap();
        assert!(session.is_dirty());
        let mut app = FilmcraftApp::new(session).with_presentation(Presentation::LtsEmbedded);
        let ctx = Context::default();
        for _ in 0..2 {
            app.auto.begin_frame();
            let mut output =
                ctx.run_ui(RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 720.0))), ..Default::default() }, |ui| {
                    filmcraft_ui_egui::header::show(&mut app, ui, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 38.0)));
                });
            output.textures_delta.clear();
        }
        assert!(app.auto.find("header.discord").is_none());
        assert!(app.auto.find("header.workspaces").is_some());
        let title = app.auto.find("header.documentTitle");
        if width >= 1280.0 {
            assert!(title.is_some(), "dirty title must have space at {width}px");
        }
        if let Some(title) = title {
            assert_eq!(title.label, "LTS title geometry - Edited");
            let [x, y, w, h] = title.rect;
            assert!(w > 0.0 && h > 0.0 && h < 38.0, "actual galley bounds, not full header gap: {:?}", title.rect);
            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
            assert!(x >= 0.0 && x + w <= width && y >= 0.0 && y + h <= 38.0);
            for control in app.auto.elements.iter().filter(|element| element.id.starts_with("header.") && element.id != "header.documentTitle") {
                let [cx, cy, cw, ch] = control.rect;
                let control_rect = egui::Rect::from_min_size(egui::pos2(cx, cy), egui::vec2(cw, ch));
                assert!(!rect.intersects(control_rect), "title overlaps {} at {width}px", control.id);
            }
        }
    }
}

// Append after the staged EmbeddedDriver tests. This drives an actual native keyboard
// frame and checks FullOutput; it does not merely assert a policy helper's return value.
#[test]
fn rebound_community_shortcut_opens_only_in_standalone_mode() {
    for presentation in [Presentation::Standalone, Presentation::LtsEmbedded] {
        let (tx, rx) = std::sync::mpsc::channel();
        let app = FilmcraftApp::new(Session::default()).with_presentation(presentation).with_control(rx);
        let harness = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = EmbeddedDriver { harness, tx };
        d.frames(4);
        d.ok("engine.execute", json!({"command": "shortcuts.set", "params": {"command": "help.discord", "keys": "F12"}}));
        d.harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::F12,
            physical_key: Some(egui::Key::F12),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        d.frames(1);
        let urls: Vec<_> = d
            .harness
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|command| if let OutputCommand::OpenUrl(open) = command { Some(open.url.as_str()) } else { None })
            .collect();
        if presentation == Presentation::Standalone {
            assert_eq!(urls, vec![links::DISCORD]);
        } else {
            assert!(urls.is_empty(), "embedded rebound key produced external navigation: {urls:?}");
        }
        assert!(d.harness.state().session.shortcuts.bindings.iter().any(|binding| binding.command == "help.discord" && binding.keys == "F12"));
    }
}

// Append to embedded_presentation_ui.rs. Drives real keyboard frames; no policy-helper assertions.
fn precedence_driver(presentation: Presentation) -> EmbeddedDriver {
    let (tx, rx) = std::sync::mpsc::channel();
    let app = FilmcraftApp::new(Session::default()).with_presentation(presentation).with_control(rx);
    let harness = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
    let mut driver = EmbeddedDriver { harness, tx };
    driver.frames(4);
    driver
}

fn precedence_key(driver: &mut EmbeddedDriver, key: egui::Key, modifiers: egui::Modifiers, pressed: bool) -> Vec<String> {
    driver.harness.input_mut().events.push(egui::Event::ModifiersChanged(modifiers));
    driver.harness.input_mut().events.push(egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers });
    driver.frames(1);
    driver
        .harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn hidden_specific_shortcut_absorbs_modifiers_without_firing_less_specific_tool() {
    use filmcraft_ui_egui::state::Tool;
    for (chord, modifiers) in [("Shift+F12", egui::Modifiers::SHIFT), ("Alt+Shift+F12", egui::Modifiers { alt: true, shift: true, ..egui::Modifiers::NONE })] {
        for presentation in [Presentation::Standalone, Presentation::LtsEmbedded] {
            let mut d = precedence_driver(presentation);
            d.ok("engine.execute", json!({"command": "shortcuts.set", "params": {"command": "tool.razor", "keys": "F12"}}));
            d.ok("engine.execute", json!({"command": "shortcuts.set", "params": {"command": "help.github", "keys": chord}}));
            let original = serde_json::to_value(&d.harness.state().session.shortcuts.bindings).unwrap();
            assert_eq!(d.harness.state().ui.tool, Tool::Selection);
            let urls = precedence_key(&mut d, egui::Key::F12, modifiers, true);
            assert_eq!(d.harness.state().ui.tool, Tool::Selection, "{presentation:?}: hidden {chord} must absorb the chord before plain F12");
            if presentation == Presentation::Standalone {
                assert_eq!(urls, vec![links::GITHUB.to_string()]);
            } else {
                assert!(urls.is_empty(), "hidden shortcut opened URL: {urls:?}");
                assert_eq!(d.harness.state().ui.status, "Community navigation is unavailable in the LTS editor");
            }
            assert_eq!(serde_json::to_value(&d.harness.state().session.shortcuts.bindings).unwrap(), original);
            precedence_key(&mut d, egui::Key::F12, modifiers, false);
            let urls = precedence_key(&mut d, egui::Key::F12, egui::Modifiers::NONE, true);
            assert_eq!(d.harness.state().ui.tool, Tool::Razor, "unmodified F12 remains a working manual command");
            assert!(urls.is_empty());
        }
    }
}

#[test]
fn hidden_panel_shortcut_absorbs_key_without_firing_application_tool() {
    use filmcraft_ui_egui::dock::PanelKind;
    use filmcraft_ui_egui::state::Tool;
    for presentation in [Presentation::Standalone, Presentation::LtsEmbedded] {
        let mut d = precedence_driver(presentation);
        d.ok("engine.execute", json!({"command": "shortcuts.set", "params": {"command": "tool.razor", "keys": "K"}}));
        d.ok("engine.execute", json!({"command": "shortcuts.set", "params": {"command": "help.discord", "keys": "K", "panel": "Timeline"}}));
        let original = serde_json::to_value(&d.harness.state().session.shortcuts.bindings).unwrap();
        d.harness.state_mut().ui.focused = PanelKind::Timeline;
        assert_eq!(d.harness.state().ui.tool, Tool::Selection);
        let urls = precedence_key(&mut d, egui::Key::K, egui::Modifiers::NONE, true);
        assert_eq!(d.harness.state().ui.tool, Tool::Selection, "{presentation:?}: Timeline's hidden K must absorb the key before application K");
        if presentation == Presentation::Standalone {
            assert_eq!(urls, vec![links::DISCORD.to_string()]);
        } else {
            assert!(urls.is_empty(), "hidden panel shortcut opened URL: {urls:?}");
            assert_eq!(d.harness.state().ui.status, "Community navigation is unavailable in the LTS editor");
        }
        assert_eq!(serde_json::to_value(&d.harness.state().session.shortcuts.bindings).unwrap(), original);
        precedence_key(&mut d, egui::Key::K, egui::Modifiers::NONE, false);
        d.harness.state_mut().ui.focused = PanelKind::Project;
        let urls = precedence_key(&mut d, egui::Key::K, egui::Modifiers::NONE, true);
        assert_eq!(d.harness.state().ui.tool, Tool::Razor, "application K must remain usable outside Timeline");
        assert!(urls.is_empty());
    }
}

#[test]
fn explicit_reassignment_from_hidden_shortcut_has_readable_message_and_cancel_restores_bindings() {
    let mut d = precedence_driver(Presentation::LtsEmbedded);
    d.ok("engine.execute", json!({"command": "shortcuts.set", "params": {"command": "help.discord", "keys": "F12"}}));
    let original = serde_json::to_value(&d.harness.state().session.shortcuts.bindings).unwrap();
    d.ok("engine.execute", json!({"command": "app.keyboardShortcuts"}));
    d.harness.state_mut().shortcut_editor.search = "Razor".into();
    d.frames(3);
    assert_eq!(d.ids("shortcuts.cell.tool.razor"), vec!["shortcuts.cell.tool.razor"]);
    d.ok("ui.click", json!({"id": "shortcuts.cell.tool.razor"}));
    assert_eq!(d.harness.state().shortcut_editor.recording, Some(("tool.razor".to_string(), false)));
    let urls = precedence_key(&mut d, egui::Key::F12, egui::Modifiers::NONE, true);
    assert!(urls.is_empty(), "recording a key must not dispatch the hidden command");
    assert_eq!(d.harness.state().shortcut_editor.message, "F12 → Razor Tool (removed from an unavailable command)");
    assert!(
        d.harness.state().session.shortcuts.bindings.iter().any(|binding| binding.command == "tool.razor" && binding.keys == "F12" && binding.panel.is_none())
    );
    assert!(
        !d.harness
            .state()
            .session
            .shortcuts
            .bindings
            .iter()
            .any(|binding| binding.command == "help.discord" && binding.keys == "F12" && binding.panel.is_none())
    );
    precedence_key(&mut d, egui::Key::F12, egui::Modifiers::NONE, false);
    d.ok("ui.click", json!({"id": "shortcuts.cancel"}));
    d.frames(2);
    assert!(d.harness.state().dialog.is_none());
    assert_eq!(serde_json::to_value(&d.harness.state().session.shortcuts.bindings).unwrap(), original);
    assert!(d.harness.output().platform_output.commands.iter().all(|command| !matches!(command, OutputCommand::OpenUrl(_))));
}

// LTS rebranding: embedded surfaces name no upstream product, community or repository and open no
// external URL; standalone keeps the upstream wording (regression controls).

/// Upstream product, community and repository names an embedded surface must not show.
const UPSTREAM_BRANDING: [&str; 5] = ["FilmCraft", "ArtCraft", "Discord", "getartcraft", "storytold"];
/// The plain-text attribution the embedded About keeps.
const ATTRIBUTION: &str = "Modified version based on FilmCraft by the ArtCraft team and community.";

fn upstream_branding(text: &str) -> Option<&'static str> {
    UPSTREAM_BRANDING.into_iter().find(|brand| text.contains(brand))
}

fn assert_product_neutral(text: &str, context: &str) {
    assert!(upstream_branding(text).is_none(), "{context}: upstream branding in {text:?}");
}

/// Every widget text the last frame exposes to AccessKit (labels, values and descriptions), each
/// whole. The per-row text runs of a wrapped label are skipped: they are pieces of their widget's
/// text, which its own node carries complete.
fn accessible_text(harness: &egui_kittest::Harness<'_, FilmcraftApp>) -> Vec<String> {
    harness
        .query_all_by(|node| node.role() != egui::accesskit::Role::TextRun)
        .flat_map(|node| {
            let node = node.accesskit_node();
            [node.label(), node.value(), node.description()]
        })
        .flatten()
        .collect()
}

impl EmbeddedDriver {
    fn text(&self) -> Vec<String> {
        accessible_text(&self.harness)
    }

    /// Visible text naming an upstream product, except the retained attribution sentence and the
    /// "FilmCraft Previews" cache folder (a storage path, kept so native preview caches survive).
    fn upstream_text(&self) -> Vec<String> {
        self.text().into_iter().filter(|text| upstream_branding(text).is_some() && text != ATTRIBUTION && !text.contains("FilmCraft Previews")).collect()
    }

    /// The `OpenUrl` requests (url, new tab) of the last frame.
    fn open_urls(&self) -> Vec<(String, bool)> {
        self.harness
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                OutputCommand::OpenUrl(open) => Some((open.url.clone(), open.new_tab)),
                _ => None,
            })
            .collect()
    }

    /// `ok`, also collecting the `OpenUrl` requests of every frame until the reply and two after it.
    fn ok_with_urls(&mut self, method: &str, params: Value) -> (Value, Vec<(String, bool)>) {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).expect("control inbox stays alive");
        let mut urls = Vec::new();
        for _ in 0..600 {
            self.frames(1);
            urls.extend(self.open_urls());
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], json!(true), "{method} {params}: {value}");
                for _ in 0..2 {
                    self.frames(1);
                    urls.extend(self.open_urls());
                }
                return (value["result"].clone(), urls);
            }
        }
        panic!("no bounded control reply for {method} {params}");
    }

    fn open_about_tab(&mut self, tab: &str) {
        self.ok("engine.execute", json!({"command": "app.about"}));
        self.frames(2);
        self.ok("ui.click", json!({"id": format!("about.tab.{tab}")}));
        self.frames(3);
    }
}

#[test]
fn embedded_contributor_names_never_navigate() {
    let mut d = EmbeddedDriver::new();
    let project = d.harness.state().session.project.clone();
    d.open_about_tab("contributors");
    let person = d.ids("about.credits.person.").into_iter().next().expect("the About window lists the compiled-in contributors");
    let (_, urls) = d.ok_with_urls("ui.click", json!({"id": person}));
    assert!(urls.is_empty(), "a contributor name must not navigate the embedded editor: {urls:?}");
    assert_eq!(d.harness.query_all_by_role(egui::accesskit::Role::Link).count(), 0, "embedded credits are plain text, not links");
    assert!(!d.ids("about.tab.contributors").is_empty(), "the About window stays open");
    assert!(std::sync::Arc::ptr_eq(&project, &d.harness.state().session.project), "the working project is untouched");
}

#[test]
fn standalone_contributor_link_opens_a_new_tab() {
    let mut d = precedence_driver(Presentation::Standalone);
    d.open_about_tab("contributors");
    let person = d.ids("about.credits.person.").into_iter().next().expect("the About window lists the compiled-in contributors");
    let login = person.trim_start_matches("about.credits.person.").to_string();
    let (_, urls) = d.ok_with_urls("ui.click", json!({"id": person}));
    assert_eq!(urls, vec![(format!("https://github.com/{login}"), true)], "a profile opens in a new tab, never in the editor's own tab");
}

#[test]
fn embedded_preferences_have_no_upstream_help() {
    for presentation in [Presentation::Standalone, Presentation::LtsEmbedded] {
        let mut d = precedence_driver(presentation);
        d.ok("engine.execute", json!({"command": "app.settings.general"}));
        d.frames(3);
        let ids = d.ids("settings.");
        for kept in ["settings.ok", "settings.cancel", "settings.reset"] {
            assert!(ids.iter().any(|id| id == kept), "{presentation:?}: {kept} in {ids:?}");
        }
        assert_eq!(ids.iter().any(|id| id == "settings.help"), presentation == Presentation::Standalone, "{presentation:?}: upstream documentation button");
    }
}

#[test]
fn embedded_menu_and_shortcut_labels_are_product_neutral() {
    let mut d = EmbeddedDriver::new();
    let menu = d.ok("ui.menu.list", json!({}));
    let rows = menu.as_array().expect("menu rows");
    for row in rows {
        assert_product_neutral(row["label"].as_str().unwrap_or_default(), &format!("menu {}", row["id"]));
    }
    let export = rows.iter().find(|row| row["id"] == "file.exportSelectionProject").expect("File ▸ Export keeps the selection export");
    assert_eq!(export["label"], "Selection as Project…");
    d.ok("engine.execute", json!({"command": "app.keyboardShortcuts"}));
    for search in ["Quit", "Selection", "Help", "About"] {
        d.harness.state_mut().shortcut_editor.search = search.into();
        d.frames(3);
        for row in d.elements("shortcuts.row.") {
            assert_product_neutral(row["label"].as_str().unwrap_or_default(), &format!("shortcut row {} ({search})", row["id"]));
        }
    }
    d.harness.state_mut().shortcut_editor.search = "Quit".into();
    d.frames(3);
    let quit = d.elements("shortcuts.row.app.quit");
    assert_eq!(quit.len(), 1, "quit keeps its row and key");
    assert_eq!(quit[0]["label"], "Quit editor");

    // standalone keeps the upstream wording
    let mut s = precedence_driver(Presentation::Standalone);
    let menu = s.ok("ui.menu.list", json!({}));
    let export = menu.as_array().expect("menu rows").iter().find(|row| row["id"] == "file.exportSelectionProject").cloned().expect("selection export");
    assert_eq!(export["label"], "Selection as FilmCraft Project…");
}

#[test]
fn embedded_shortcut_listings_hide_promotional_labels_but_keep_bindings() {
    let standalone = FilmcraftApp::new(Session::default());
    let embedded = FilmcraftApp::new(Session::default()).with_presentation(Presentation::LtsEmbedded);
    assert_eq!(
        serde_json::to_value(&standalone.session.shortcuts.bindings).unwrap(),
        serde_json::to_value(&embedded.session.shortcuts.bindings).unwrap(),
        "presentation never changes default or saved bindings"
    );
    let mut d = EmbeddedDriver::new();
    let rows = d.ok("engine.execute", json!({"command": "shortcuts.list", "params": {}}));
    let rows = rows.as_array().expect("shortcut rows").clone();
    for row in &rows {
        assert_product_neutral(row["label"].as_str().unwrap_or_default(), &format!("shortcuts.list {}", row["id"]));
    }
    for (id, _, _) in links::ALL {
        assert!(rows.iter().any(|row| row["id"] == id), "{id} stays listed: a saved shortcut keeps its key");
        let row = d.ok("engine.execute", json!({"command": "shortcuts.get", "params": {"command": id}}));
        assert_eq!(row["label"], "Unavailable in this editor", "{id}");
    }
    for (id, label) in [("app.about", "Editing engine information"), ("help.filmcraftHelp", "Editing shortcuts"), ("app.quit", "Quit editor")] {
        let row = d.ok("engine.execute", json!({"command": "shortcuts.get", "params": {"command": id}}));
        assert_eq!(row["label"], label, "{id}");
    }

    // standalone keeps the upstream labels
    let mut s = precedence_driver(Presentation::Standalone);
    let row = s.ok("engine.execute", json!({"command": "shortcuts.get", "params": {"command": "help.discord"}}));
    assert_eq!(row["label"], "Join the ArtCraft Discord…");
}

#[test]
fn embedded_shortcuts_preset_is_product_neutral() {
    let suggested = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let names = suggested.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = FilmcraftApp::new(Session::default()).with_presentation(Presentation::LtsEmbedded).with_control(rx);
    app.hooks.pick_save_as = Some(Box::new(move |_filter: &str, _extensions: &[&str], name: &str| -> Option<String> {
        names.lock().unwrap().push(name.to_string());
        None
    }));
    let harness = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
    let mut d = EmbeddedDriver { harness, tx };
    d.frames(4);
    d.ok("engine.execute", json!({"command": "help.filmcraftHelp"}));
    d.frames(3);
    assert_eq!(d.upstream_text(), Vec::<String>::new(), "Editing shortcuts dialog");
    assert!(d.text().iter().any(|text| text == "Default"), "the active built-in preset reads Default");
    d.ok("ui.click", json!({"id": "shortcuts.preset"}));
    d.frames(2);
    let item = d.elements("shortcuts.preset.FilmCraft Default");
    assert_eq!(item.len(), 1, "the automation id keeps the stored preset name: {item:?}");
    assert_eq!(item[0]["label"], "Default");
    d.ok("ui.click", json!({"id": "shortcuts.preset.FilmCraft Default"}));
    d.frames(2);
    assert_eq!(d.harness.state().shortcut_editor.message, "Loaded “Default”");
    d.ok("ui.click", json!({"id": "shortcuts.export"}));
    d.frames(2);
    assert_eq!(*suggested.lock().unwrap(), vec!["Default.json".to_string()], "suggested export file name");
    let presets = d.ok("engine.execute", json!({"command": "shortcuts.presets"}));
    assert_eq!(presets["active"], "FilmCraft Default", "the stored preset name is unchanged");
}

#[test]
fn embedded_system_report_heading_is_neutral() {
    let version = env!("CARGO_PKG_VERSION");
    for presentation in [Presentation::Standalone, Presentation::LtsEmbedded] {
        let mut d = precedence_driver(presentation);
        let opened = d.ok("engine.execute", json!({"command": "help.systemCompatibilityReport"}));
        assert_eq!(opened["report"]["app"], format!("FilmCraft {version}"), "the report data keeps its diagnostic app field");
        d.frames(3);
        let text = d.text();
        if presentation == Presentation::Standalone {
            assert!(text.contains(&format!("FilmCraft {version}")), "standalone heading: {text:?}");
        } else {
            assert!(text.contains(&format!("LTS Video editor · engine {version}")), "embedded heading: {text:?}");
            assert_eq!(d.upstream_text(), Vec::<String>::new(), "System Compatibility Report");
        }
    }
}

#[test]
fn embedded_renderer_labels_are_neutral() {
    use filmcraft_engine::project_tools::{RENDERER_GPU, RENDERER_SOFTWARE};
    let mut d = EmbeddedDriver::new();
    assert_eq!(d.harness.state().session.project.settings.renderer, RENDERER_GPU);
    d.ok("engine.execute", json!({"command": "file.projectSettings.general"}));
    d.frames(3);
    let text = d.text();
    assert!(text.iter().any(|t| t == "GPU Acceleration (wgpu)"), "renderer without the product name: {text:?}");
    assert_eq!(d.upstream_text(), Vec::<String>::new(), "Project Settings");
    d.ok("ui.set", json!({"menuDialog": {"renderer": RENDERER_SOFTWARE}}));
    d.frames(2);
    assert!(d.text().iter().any(|t| t == "Software Only (CPU)"));
    assert_eq!(d.upstream_text(), Vec::<String>::new(), "Project Settings, software renderer");
    d.ok("ui.click", json!({"id": "projectSettings.ok"}));
    d.frames(3);
    assert_eq!(d.harness.state().session.project.settings.renderer, RENDERER_SOFTWARE, "the stored renderer value is unchanged");
}

#[test]
fn embedded_surface_sweep_has_no_upstream_branding() {
    let none = Vec::<String>::new();
    let mut d = EmbeddedDriver::new();
    assert_eq!(d.upstream_text(), none, "Edit mode");
    d.ok("engine.execute", json!({"command": "app.about"}));
    d.frames(3);
    let text = d.text();
    assert!(text.iter().any(|t| t == "LTS Video editor"), "About heading: {text:?}");
    assert_eq!(text.iter().filter(|t| *t == ATTRIBUTION).count(), 1, "exactly one plain-text attribution: {text:?}");
    assert_eq!(d.upstream_text(), none, "About");
    for tab in ["contributors", "models"] {
        d.ok("ui.click", json!({"id": format!("about.tab.{tab}")}));
        d.frames(3);
        assert_eq!(d.upstream_text(), none, "About ▸ {tab}");
    }
    d.ok("ui.key", json!({"key": "Escape"}));
    d.frames(3);
    d.ok("engine.execute", json!({"command": "help.filmcraftHelp"}));
    d.frames(3);
    assert_eq!(d.upstream_text(), none, "Editing shortcuts");
    d.ok("ui.click", json!({"id": "shortcuts.cancel"}));
    d.frames(3);
    for page in ["general", "autoSave", "mediaCache", "memory", "playback", "plugins"] {
        d.ok("engine.execute", json!({"command": format!("app.settings.{page}")}));
        d.frames(3);
        assert_eq!(d.upstream_text(), none, "Preferences ▸ {page}");
        d.ok("ui.click", json!({"id": "settings.cancel"}));
        d.frames(3);
    }
    for mode in ["mode.import", "mode.export", "mode.edit"] {
        d.ok("engine.execute", json!({"command": mode}));
        d.frames(3);
        assert_eq!(d.upstream_text(), none, "{mode}");
    }
    d.ok("engine.execute", json!({"command": "help.systemCompatibilityReport"}));
    d.frames(3);
    assert_eq!(d.upstream_text(), none, "System Compatibility Report");
}

#[test]
fn about_shows_third_party_notices_as_plain_text_in_both_modes() {
    for presentation in [Presentation::Standalone, Presentation::LtsEmbedded] {
        let mut d = precedence_driver(presentation);
        d.open_about_tab("notices");
        let text = d.text().join("\n");
        assert!(text.contains(ATTRIBUTION), "{presentation:?}: attribution");
        assert!(text.contains("Cargo.lock") && text.contains("pending"), "{presentation:?}: the notices must not claim completeness");
        assert!(text.contains("Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors"), "{presentation:?}: NOTICE is reproduced");
        let notices = d.ids("about.notices.");
        for id in ["notice", "licenseMit", "licenseApache", "attribution", "oflInter", "oflNotoSerif", "oflJetBrainsMono", "eguiDefaultFonts"] {
            assert!(notices.contains(&format!("about.notices.{id}")), "{presentation:?}: {id} in {notices:?}");
        }
        let (_, urls) = d.ok_with_urls("ui.click", json!({"id": "about.notices.eguiDefaultFonts"}));
        d.frames(2);
        let text = d.text().join("\n");
        for licence in
            ["UBUNTU FONT LICENCE Version 1.0", "SIL OPEN FONT LICENSE Version 1.1", "Copyright 2018 Source Foundry Authors", "Copyright (c) 2014 John Slegers"]
        {
            assert!(text.contains(licence), "{presentation:?}: the egui default-font notice lacks {licence}");
        }
        assert!(urls.is_empty(), "{presentation:?}: {urls:?}");
        assert_eq!(d.harness.query_all_by_role(egui::accesskit::Role::Link).count(), 0, "{presentation:?}: notices are plain text");
    }
}

#[test]
fn embedded_reveal_commands_open_no_file_url() {
    // A saved project; like the web build, these apps install no host `open_path` hook, so a reveal
    // falls back to a `file://` URL.
    let saved = "/projects/Untitled.fcproj";
    for presentation in [Presentation::Standalone, Presentation::LtsEmbedded] {
        let mut app = FilmcraftApp::new(Session::default()).with_presentation(presentation);
        app.session.path = Some(saved.into());
        let ctx = Context::default();
        let (reply, opened) = invoke(&mut app, &ctx, "engine.execute", "projectPanel.revealProject");
        if presentation == Presentation::Standalone {
            assert_eq!(reply, json!({"ok": true, "result": {"path": saved}}));
            assert_eq!(opened, vec![format!("file://{saved}")], "the standalone app still opens the file URL");
        } else {
            assert_eq!(reply, json!({"ok": false, "error": "External links are unavailable in the LTS editor"}));
            assert!(opened.is_empty(), "embedded reveal enqueued {opened:?}");
        }
    }
    // Reveal Log Files has no file manager to reveal in either: refused before a log is written
    let mut app = FilmcraftApp::new(Session::default()).with_presentation(Presentation::LtsEmbedded);
    let ctx = Context::default();
    let (reply, opened) = invoke(&mut app, &ctx, "engine.execute", "help.revealLogFiles");
    assert_eq!(reply, json!({"ok": false, "error": "External links are unavailable in the LTS editor"}));
    assert!(opened.is_empty(), "{opened:?}");
}

#[test]
fn embedded_engine_command_catalog_uses_menu_labels() {
    for (presentation, label) in [(Presentation::Standalone, "Selection as FilmCraft Project…"), (Presentation::LtsEmbedded, "Selection as Project…")] {
        let mut d = precedence_driver(presentation);
        let rows = d.ok("engine.commands", json!({}));
        let row = rows
            .as_array()
            .expect("command rows")
            .iter()
            .find(|row| row["id"] == "file.exportSelectionProject")
            .cloned()
            .expect("the selection export is an engine command");
        assert_eq!(row["label"], label, "{presentation:?}");
        assert_eq!(row["menu"], json!(["File", "Export"]), "{presentation:?}: only the label follows the presentation");
    }
}
