use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, FontFamily, FontId, Frame, Label, Layout, Margin,
    RichText, ScrollArea, Stroke, TextEdit, TextStyle, Vec2,
};

use crate::fontmap::{self, FontItem};
use crate::game;
use crate::lpk::{self, Archive, BuildSummary, PROFILES, PackageEntry, Profile};
use crate::settings::{self, FontSetting, Settings};

const VERSION: &str = "0.2.0";
const ACCENT: Color32 = Color32::from_rgb(124, 92, 255);
const BG: Color32 = Color32::from_rgb(13, 15, 21);
const PANEL: Color32 = Color32::from_rgb(17, 20, 28);
const CARD: Color32 = Color32::from_rgb(22, 26, 36);
const INPUT: Color32 = Color32::from_rgb(28, 33, 45);
const HOVER: Color32 = Color32::from_rgb(35, 41, 56);
const BORDER: Color32 = Color32::from_rgb(42, 48, 64);
const TEXT: Color32 = Color32::from_rgb(231, 234, 242);
const MUTED: Color32 = Color32::from_rgb(139, 146, 164);
const GREEN: Color32 = Color32::from_rgb(64, 196, 132);
const ORANGE: Color32 = Color32::from_rgb(235, 168, 76);
const RED: Color32 = Color32::from_rgb(232, 92, 100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Fonts,
    BuildInstall,
    Extract,
    Info,
}

#[derive(Clone, Copy, Debug)]
struct Language {
    label: &'static str,
    folder: &'static str,
}

const LANGUAGES: &[Language] = &[
    Language {
        label: "English",
        folder: r"\Binaries\Fonts\English",
    },
    Language {
        label: "Spanish",
        folder: r"\Binaries\Fonts\Spanish",
    },
    Language {
        label: "French",
        folder: r"\Binaries\Fonts\French",
    },
    Language {
        label: "German",
        folder: r"\Binaries\Fonts\German",
    },
    Language {
        label: "Russian",
        folder: r"\Binaries\Fonts\Russian",
    },
    Language {
        label: "Japanese",
        folder: r"\Binaries\Fonts\Japanese",
    },
    Language {
        label: "Chinese",
        folder: r"\Binaries\Fonts\Chinese",
    },
    Language {
        label: "Taiwanese",
        folder: r"\Binaries\Fonts\Taiwanese",
    },
    Language {
        label: "Korean (Root)",
        folder: r"\Binaries\Fonts",
    },
];

enum Msg {
    Log(String),
    Done(Result<Outcome>),
}

enum Outcome {
    ArchiveLoaded {
        path: PathBuf,
        archive: Arc<Archive>,
    },
    Extracted(usize),
    Built {
        path: PathBuf,
        summary: BuildSummary,
        installed: bool,
    },
    Installed,
    Reverted,
    FontMapImported(Vec<FontItem>),
    FontsImported {
        fonts: Vec<FontSetting>,
        skipped: usize,
    },
}

pub struct FontStudioApp {
    page: Page,
    profile: Profile,
    language: String,
    fonts: Vec<FontSetting>,
    fontmap_rows: Vec<FontItem>,
    output_path: Option<PathBuf>,
    last_built: Option<PathBuf>,
    game_dir: Option<PathBuf>,
    archive_path: Option<PathBuf>,
    archive: Option<Arc<Archive>>,
    filter: String,
    rename_origin: Option<(usize, String)>,
    log: Vec<String>,
    busy: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl FontStudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        let settings = settings::load();
        let (tx, rx) = mpsc::channel();

        let profile = settings
            .profile
            .as_deref()
            .map(Profile::from_label)
            .unwrap_or_default();
        let language = settings
            .language
            .filter(|label| {
                LANGUAGES
                    .iter()
                    .any(|language| language.label == label.as_str())
            })
            .unwrap_or_else(|| LANGUAGES[0].label.to_string());

        let mut fonts = settings.fonts;
        fonts.retain(|font| !font.path.as_os_str().is_empty());
        for font in &mut fonts {
            if font.name.trim().is_empty() {
                font.name = file_name(&font.path);
            }
        }

        let mut fontmap_rows = settings.fontmap_rows;
        if fontmap_rows.is_empty()
            && let Some(font) = fonts.first()
        {
            fontmap_rows = default_rows(font.name.trim());
        }

        let game_dir = settings
            .game_dir
            .filter(|path| path.is_dir())
            .or_else(game::find_game_dir);

        let mut output = settings.output;
        if let (Some(path), Some(dir)) = (&output, &game_dir)
            && same_path(path, &dir.join("font.lpk"))
        {
            output = None;
        }
        let output = output.or_else(|| Some(default_output(game_dir.as_deref())));

        let mut app = Self {
            page: Page::Fonts,
            profile,
            language,
            fonts,
            fontmap_rows,
            output_path: output,
            last_built: None,
            game_dir,
            archive_path: None,
            archive: None,
            filter: String::new(),
            rename_origin: None,
            log: Vec::new(),
            busy: false,
            tx,
            rx,
        };

        app.sync_fontmap_to_fonts();

        if let Some(path) = settings.archive.filter(|path| path.is_file()) {
            app.load_archive(path);
        }

        app
    }

    fn poll_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Msg::Log(line) => self.log.push(line),
                Msg::Done(result) => {
                    self.busy = false;
                    match result {
                        Ok(outcome) => self.handle_outcome(outcome),
                        Err(error) => {
                            let text = format!("{error:#}");
                            self.log.push(format!("Error: {text}"));
                        }
                    }
                }
            }
        }
    }

    fn handle_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::ArchiveLoaded { path, archive } => {
                let count = archive.entries.len();
                self.archive_path = Some(path.clone());
                self.archive = Some(archive);
                self.log(format!(
                    "Opened {} ({count} entries, {})",
                    path.display(),
                    self.profile.label
                ));
            }
            Outcome::Extracted(count) => {
                self.log(format!("Extracted {count} files."));
            }
            Outcome::Built {
                path,
                summary,
                installed,
            } => {
                self.log(format!(
                    "Built {} ({} entries, {:.1} MB)",
                    path.display(),
                    summary.entries,
                    summary.bytes as f64 / 1_048_576.0
                ));
                self.last_built = Some(path.clone());
                self.output_path = Some(path);
                if installed {
                    self.log(format!(
                        "Built and installed {} entries ({:.1} MB). The original was saved as font.lpk.bak.",
                        summary.entries,
                        summary.bytes as f64 / 1_048_576.0
                    ));
                } else {
                    self.log(format!(
                        "Built {} entries ({:.1} MB). Use Install built package when you are ready.",
                        summary.entries,
                        summary.bytes as f64 / 1_048_576.0
                    ));
                }
            }
            Outcome::Installed => {
                self.log("Modded font installed. The original was saved as font.lpk.bak.");
            }
            Outcome::Reverted => {
                self.log("The original font.lpk was restored.");
            }
            Outcome::FontMapImported(rows) => {
                let count = rows.len();
                if count == 0 {
                    self.report_error("The game's FontMap has no rows.");
                } else {
                    self.fontmap_rows = rows;
                    self.sync_fontmap_to_fonts();
                    self.log(format!(
                        "Imported {count} FontMap rows from the game archive."
                    ));
                }
            }
            Outcome::FontsImported { fonts, skipped } => {
                let count = fonts.len();
                self.fonts.extend(fonts);
                if count == 0 {
                    self.report_error(format!(
                        "No fonts were imported from the {} folder in the game archive.",
                        self.selected_language().label
                    ));
                } else {
                    self.log(format!(
                        "Imported {count} font file(s) from the game archive into the font list."
                    ));
                }
                if skipped > 0 {
                    self.log(format!(
                        "Skipped {skipped} font file(s) already present in the font list."
                    ));
                }
            }
        }
    }

    fn spawn<F>(&mut self, job: F)
    where
        F: FnOnce(Sender<Msg>) -> Result<Outcome> + Send + 'static,
    {
        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = job(tx.clone());
            let _ = tx.send(Msg::Done(result));
        });
    }

    fn log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
    }

    fn report_error(&mut self, message: impl Into<String>) {
        self.log(format!("Error: {}", message.into()));
    }

    fn selected_language(&self) -> Language {
        LANGUAGES
            .iter()
            .copied()
            .find(|language| language.label == self.language.as_str())
            .unwrap_or(LANGUAGES[0])
    }

    fn fontmap_path(&self) -> String {
        format!("{}\\FontMap.xml", self.selected_language().folder)
    }

    fn has_game(&self) -> bool {
        self.game_dir
            .as_ref()
            .is_some_and(|dir| dir.join("font.lpk").is_file())
    }

    fn game_archive_path(&self) -> Option<PathBuf> {
        let dir = self.game_dir.as_ref()?;
        let paths = game::paths(dir);
        if paths.backup.is_file() {
            Some(paths.backup)
        } else if paths.font.is_file() {
            Some(paths.font)
        } else {
            None
        }
    }

    fn built_archive(&self) -> Option<PathBuf> {
        self.last_built
            .clone()
            .filter(|path| path.is_file())
            .or_else(|| self.output_path.clone().filter(|path| path.is_file()))
    }

    fn sync_fontmap_to_fonts(&mut self) {
        sync_rows_to_fonts(&mut self.fontmap_rows, &self.fonts);
    }

    fn package_problem(&self) -> Option<String> {
        if self.fonts.is_empty() {
            return Some("Add at least one font file.".to_string());
        }

        let mut names = HashSet::new();
        for font in &self.fonts {
            let name = font.name.trim();
            if !valid_font_name(name) {
                return Some(format!(
                    "'{name}' is not a valid file name. Use a plain file name without folders."
                ));
            }
            if !font.path.is_file() {
                return Some(format!("Font file not found: {}", font.path.display()));
            }
            if !names.insert(name.to_ascii_lowercase()) {
                return Some(format!("Two fonts use the archive name '{name}'."));
            }
        }

        if self.fontmap_rows.is_empty() {
            return Some(
                "The FontMap has no rows. Add a row or load the defaults from the game."
                    .to_string(),
            );
        }
        if let Err(error) = fontmap::validate(&self.fontmap_rows) {
            return Some(format!("FontMap: {error}"));
        }
        for row in &self.fontmap_rows {
            if !names.contains(&row.file.trim().to_ascii_lowercase()) {
                return Some(format!(
                    "FontMap row '{}' points to '{}', which is not in the font list.",
                    row.key.trim(),
                    row.file.trim()
                ));
            }
        }

        match &self.output_path {
            Some(output) => {
                if let Some(game_dir) = &self.game_dir
                    && same_path(output, &game_dir.join("font.lpk"))
                {
                    return Some(
                        "Choose an output file other than the game's font.lpk.".to_string(),
                    );
                }
                if output.is_dir() {
                    return Some("The output path is a folder. Choose a file name.".to_string());
                }
            }
            None => return Some("Choose an output file.".to_string()),
        }

        None
    }

    fn load_archive(&mut self, path: PathBuf) {
        let profile = self.profile;
        self.spawn(move |tx| {
            let _ = tx.send(Msg::Log(format!("Reading {}...", path.display())));
            let archive = Archive::read(&path, profile)?;
            Ok(Outcome::ArchiveLoaded {
                path,
                archive: Arc::new(archive),
            })
        });
    }

    fn open_archive_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open font.lpk")
            .add_filter("LPK archive", &["lpk", "bak"])
            .pick_file()
        {
            self.load_archive(path);
        }
    }

    fn open_game_archive(&mut self) {
        match self.game_archive_path() {
            Some(path) => {
                let kind = if path.extension().is_some_and(|ext| ext == "bak") {
                    "backup"
                } else {
                    "current"
                };
                self.log(format!("Opening game {kind} archive: {}", path.display()));
                self.load_archive(path);
            }
            None => {
                self.report_error(
                    "The game folder is not set or contains no font.lpk. Select the EFGame folder on the Fonts tab.",
                );
            }
        }
    }

    fn pick_fonts(&mut self) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Add font files")
            .add_filter("Fonts", &["ttf", "otf", "ttc"])
            .pick_files()
        else {
            return;
        };

        let first_font = self.fonts.is_empty();
        let mut added = 0;
        let mut skipped = Vec::new();
        for path in paths {
            if !font_magic(&path) {
                skipped.push(file_name(&path));
                continue;
            }
            let name = unique_font_name(&path, &self.fonts);
            self.fonts.push(FontSetting { path, name });
            added += 1;
        }

        if added > 0 {
            if first_font {
                if let Some(font) = self.fonts.first() {
                    let name = font.name.trim().to_string();
                    if self.fontmap_rows.is_empty() {
                        self.fontmap_rows = default_rows(&name);
                    } else {
                        for row in &mut self.fontmap_rows {
                            row.file = name.clone();
                        }
                    }
                }
            } else {
                self.sync_fontmap_to_fonts();
            }
        }

        if added > 0 {
            self.log(format!("Added {added} font file(s)."));
        }
        if !skipped.is_empty() {
            self.report_error(format!(
                "Skipped {}: not a TrueType/OpenType font.",
                skipped.join(", ")
            ));
        }
    }

    fn import_game_fonts(&mut self) {
        let Some(path) = self.game_archive_path() else {
            self.report_error("Select the game folder first so its original font.lpk can be read.");
            return;
        };
        let profile = self.profile;
        let language = self.selected_language();
        let target = lpk::normalize_path(language.folder);
        let cache_dir = imported_fonts_dir(language.label);
        let existing: HashSet<String> = self
            .fonts
            .iter()
            .map(|font| font.name.trim().to_ascii_lowercase())
            .collect();

        self.spawn(move |tx| {
            let _ = tx.send(Msg::Log(format!(
                "Reading fonts from {}...",
                path.display()
            )));
            let archive = Archive::read(&path, profile)?;
            let mut fonts = Vec::new();
            let mut skipped = 0;
            let mut seen = existing;

            for entry in &archive.entries {
                if lpk::normalize_path(entry_folder(&entry.name)) != target
                    || !is_font_file(&entry.name)
                {
                    continue;
                }

                let name = entry_file_name(&entry.name).to_string();
                if !seen.insert(name.to_ascii_lowercase()) {
                    skipped += 1;
                    continue;
                }

                let data = archive.extract(entry, profile)?;
                fs::create_dir_all(&cache_dir)
                    .with_context(|| format!("Could not create {}", cache_dir.display()))?;
                let output = cache_dir.join(&name);
                fs::write(&output, &data)
                    .with_context(|| format!("Could not write {}", output.display()))?;
                let _ = tx.send(Msg::Log(format!(
                    "Imported {} from the game archive.",
                    name
                )));
                fonts.push(FontSetting { path: output, name });
            }

            Ok(Outcome::FontsImported { fonts, skipped })
        });
    }

    fn pick_output(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save package as")
            .add_filter("LPK archive", &["lpk"])
            .set_file_name("font_mod.lpk");
        if let Some(dir) = self
            .output_path
            .as_ref()
            .and_then(|path| path.parent())
            .or(self.game_dir.as_deref())
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.save_file() {
            self.output_path = Some(path);
        }
    }

    fn pick_game_dir(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select the Lost Ark EFGame folder")
            .pick_folder()
        {
            if path.join("font.lpk").is_file() {
                self.game_dir = Some(path);
            } else {
                self.report_error(
                    "That folder does not contain font.lpk. Select the EFGame folder.",
                );
            }
        }
    }

    fn detect_game(&mut self) {
        match game::find_game_dir() {
            Some(path) => {
                self.log(format!("Found Lost Ark at {}", path.display()));
                self.game_dir = Some(path);
            }
            None => {
                self.report_error(
                    "Lost Ark was not found automatically. Use Browse to select the EFGame folder.",
                );
            }
        }
    }

    fn extract_all(&mut self) {
        let Some(archive) = self.archive.clone() else {
            return;
        };
        let Some(destination) = rfd::FileDialog::new()
            .set_title("Choose a folder for the extracted files")
            .pick_folder()
        else {
            return;
        };
        let profile = self.profile;
        self.spawn(move |tx| {
            let _ = tx.send(Msg::Log(format!(
                "Extracting {} entries to {}",
                archive.entries.len(),
                destination.display()
            )));
            let count = archive.extract_all(&destination, profile, |line| {
                let _ = tx.send(Msg::Log(line.to_string()));
            })?;
            Ok(Outcome::Extracted(count))
        });
    }

    fn import_fontmap(&mut self) {
        let Some(path) = self.game_archive_path() else {
            self.report_error(
                "Select the game folder first, or open an archive and edit the rows manually.",
            );
            return;
        };
        let profile = self.profile;
        let target = lpk::normalize_path(&self.fontmap_path());
        self.spawn(move |tx| {
            let _ = tx.send(Msg::Log(format!(
                "Reading FontMap from {}...",
                path.display()
            )));
            let archive = Archive::read(&path, profile)?;
            let entry = archive
                .entries
                .iter()
                .rev()
                .find(|entry| lpk::normalize_path(&entry.name) == target)
                .ok_or_else(|| {
                    anyhow!("FontMap.xml was not found for this language in the archive.")
                })?;
            let data = archive.extract(entry, profile)?;
            let rows = fontmap::parse(&data)?;
            Ok(Outcome::FontMapImported(rows))
        });
    }

    fn build(&mut self, install: bool) {
        if self.busy {
            return;
        }
        if let Some(problem) = self.package_problem() {
            self.report_error(problem);
            return;
        }
        let Some(output) = self.output_path.clone() else {
            self.report_error("Choose an output file first.");
            return;
        };
        if install && !self.has_game() {
            self.report_error("Select the game folder (EFGame) before installing.");
            return;
        }

        let fonts = self.fonts.clone();
        let rows = self.fontmap_rows.clone();
        let folder = self.selected_language().folder;
        let profile = self.profile;
        let game_dir = self.game_dir.clone();
        self.output_path = Some(output.clone());

        self.spawn(move |tx| {
            let _ = tx.send(Msg::Log(format!("Building {}...", output.display())));
            let entries = build_entries(&fonts, &rows, folder)?;
            let summary = lpk::build_package(profile, &entries, &output, |line| {
                let _ = tx.send(Msg::Log(line.to_string()));
            })?;

            if install {
                let game_dir = game_dir.context("Select the game folder before installing.")?;
                let current = game_dir.join("font.lpk");
                let scan = Archive::read(&current, profile).ok().map(|archive| {
                    archive
                        .entries
                        .iter()
                        .any(|entry| entry.name.contains("1_1_"))
                });
                let modded_now = game::current_is_modded(&game_dir, scan);
                game::install(&output, &game_dir, profile, modded_now, |line| {
                    let _ = tx.send(Msg::Log(line.to_string()));
                })?;
            }

            Ok(Outcome::Built {
                path: output,
                summary,
                installed: install,
            })
        });
    }

    fn install(&mut self, archive: PathBuf) {
        let Some(game_dir) = self
            .game_dir
            .clone()
            .filter(|dir| dir.join("font.lpk").is_file())
        else {
            self.report_error("Select the game folder (EFGame) before installing.");
            return;
        };
        let profile = self.profile;
        self.spawn(move |tx| {
            let _ = tx.send(Msg::Log(format!(
                "Installing {} into {}",
                archive.display(),
                game_dir.display()
            )));
            let current = game_dir.join("font.lpk");
            let scan = Archive::read(&current, profile).ok().map(|archive| {
                archive
                    .entries
                    .iter()
                    .any(|entry| entry.name.contains("1_1_"))
            });
            let modded_now = game::current_is_modded(&game_dir, scan);
            game::install(&archive, &game_dir, profile, modded_now, |line| {
                let _ = tx.send(Msg::Log(line.to_string()));
            })?;
            Ok(Outcome::Installed)
        });
    }

    fn pick_existing_install(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Install an existing font.lpk")
            .add_filter("LPK archive", &["lpk", "bak"])
            .pick_file()
        else {
            return;
        };
        self.install(path);
    }

    fn install_last_built(&mut self) {
        match self.built_archive() {
            Some(path) => self.install(path),
            None => {
                self.report_error("Build a package first, or choose an output file.");
            }
        }
    }

    fn revert(&mut self) {
        let Some(game_dir) = self.game_dir.clone() else {
            self.report_error("Select the game folder first.");
            return;
        };
        self.spawn(move |tx| {
            let _ = tx.send(Msg::Log(format!(
                "Restoring the original font.lpk in {}",
                game_dir.display()
            )));
            game::revert(&game_dir, |line| {
                let _ = tx.send(Msg::Log(line.to_string()));
            })?;
            Ok(Outcome::Reverted)
        });
    }

    fn settings(&self) -> Settings {
        Settings {
            profile: Some(self.profile.label.to_string()),
            language: Some(self.language.clone()),
            fonts: self.fonts.clone(),
            output: self.output_path.clone(),
            game_dir: self.game_dir.clone(),
            fontmap_rows: self.fontmap_rows.clone(),
            archive: self.archive_path.clone(),
        }
    }

    fn header(&mut self, root: &mut egui::Ui) {
        let game_found = self.has_game();
        let language = self.selected_language().label;

        egui::Panel::top("header")
            .frame(
                Frame::NONE
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(18, 14))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LOA Font Studio").size(21.0).strong());
                    ui.label(RichText::new(format!("v{VERSION}")).size(11.0).color(MUTED));
                    ui.add_space(12.0);
                    ui.label(RichText::new(language).size(12.0).color(MUTED));

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (label, color) = if self.busy {
                            ("Working...", ORANGE)
                        } else if game_found {
                            ("Game found", GREEN)
                        } else {
                            ("Game folder not set", MUTED)
                        };
                        status_chip(ui, label, color);
                    });
                });
            });
    }

    fn sidebar(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(188.0)
            .frame(
                Frame::NONE
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(12, 16))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                nav_button(ui, &mut self.page, Page::Fonts, "Fonts");
                nav_button(ui, &mut self.page, Page::BuildInstall, "Build & install");
                nav_button(ui, &mut self.page, Page::Extract, "Extract archive");
                nav_button(ui, &mut self.page, Page::Info, "Info");
            });
    }

    fn log_panel(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("activity")
            .resizable(true)
            .default_size(148.0)
            .min_size(84.0)
            .frame(
                Frame::NONE
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(16, 12))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Activity").size(13.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Clear").clicked() {
                            self.log.clear();
                        }
                    });
                });
                ui.add_space(4.0);
                ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.log.is_empty() {
                            ui.label(RichText::new("Nothing yet.").color(MUTED).size(12.0));
                        }
                        for line in &self.log {
                            ui.label(RichText::new(line).monospace().size(12.0).color(TEXT));
                        }
                    });
            });
    }

    fn fonts_page(&mut self, ui: &mut egui::Ui) {
        self.game_card(ui);
        ui.add_space(10.0);
        self.fonts_card(ui);
        ui.add_space(10.0);
        self.fontmap_card(ui);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(primary_button("Continue to Build & install"))
                    .clicked()
                {
                    self.page = Page::BuildInstall;
                }
            });
        });
    }

    fn build_install_page(&mut self, ui: &mut egui::Ui) {
        self.build_card(ui);
    }

    fn info_page(&mut self, ui: &mut egui::Ui) {
        card(ui, "How font.lpk works", |ui| {
            ui.label(
                RichText::new(
                    "font.lpk is an LPK archive that stores FontMap.xml and the font files for every language. \
                     The game loads FontMap.xml from the selected language folder and uses each Item to choose \
                     which font file handles a text style.",
                )
                .color(TEXT)
                .size(13.0),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    r"Each language uses \Binaries\Fonts\<Language>\FontMap.xml. The file at \Binaries\Fonts\FontMap.xml is for Korean",
                )
                .color(MUTED)
                .size(12.0),
            );
        });

        ui.add_space(10.0);

        card(ui, "FontMap.xml attributes", |ui| {
            egui::Grid::new("info_attributes")
                .striped(true)
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Attribute").strong());
                    ui.label(RichText::new("Meaning").strong());
                    ui.end_row();

                    for (name, description) in [
                        (
                            "Key",
                            "The text style identifier used by the game. It is not a display name.",
                        ),
                        (
                            "File",
                            "The font file name inside the same language folder. It must be included in the package.",
                        ),
                        (
                            "SizeCorrectionRatio",
                            "Font size percentage: 100 is normal, 120 is 20% larger, and 80 is 20% smaller.",
                        ),
                        (
                            "LeadingCorrection",
                            "Line spacing adjustment: 0 is normal, negative values tighten the lines, and positive values widen the gap between lines.",
                        ),
                    ] {
                        ui.label(RichText::new(name).monospace().color(ACCENT));
                        ui.label(RichText::new(description).color(TEXT).size(12.0));
                        ui.end_row();
                    }
                });
        });

        ui.add_space(10.0);

        card(ui, "Default English FontMap", |ui| {
            ui.label(
                RichText::new(
                    "These are the effective values from the current English font.lpk. Blank corrections in the original XML are shown as 100 or 0.",
                )
                .color(MUTED)
                .size(12.0),
            );
            ui.add_space(6.0);
            egui::Grid::new("info_english_defaults")
                .striped(true)
                .num_columns(5)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Key").strong());
                    ui.label(RichText::new("File").strong());
                    ui.label(RichText::new("Size").strong());
                    ui.label(RichText::new("Leading").strong());
                    ui.label(RichText::new("Role").strong());
                    ui.end_row();

                    for (key, file, size, leading, role) in [
                        (
                            "$YG760",
                            "ExuberanceF-Regular.ttf",
                            "100",
                            "0",
                            "Normal text",
                        ),
                        (
                            "$YG760_12pt",
                            "ExuberanceF-Regular.ttf",
                            "120",
                            "0",
                            "Small normal text",
                        ),
                        (
                            "$YG330",
                            "ExuberanceF-Regular.ttf",
                            "100",
                            "0",
                            "UI / normal text",
                        ),
                        (
                            "$YoonGasiIIM",
                            "MonarchaF-Regular.ttf",
                            "100",
                            "-200",
                            "Titles",
                        ),
                        (
                            "$YoonGasiIIM_Small2pt",
                            "MonarchaF-Regular.ttf",
                            "120",
                            "-200",
                            "Small titles",
                        ),
                        (
                            "$ChronologyFont",
                            "MonarchaF-Regular.ttf",
                            "100",
                            "-200",
                            "Titles / chronology",
                        ),
                        (
                            "$eventDamageFont",
                            "MonarchaF-Regular.ttf",
                            "100",
                            "0",
                            "Damage numbers",
                        ),
                    ] {
                        ui.label(RichText::new(key).monospace().color(TEXT));
                        ui.label(RichText::new(file).monospace().color(TEXT));
                        ui.label(RichText::new(size).monospace().color(MUTED));
                        ui.label(RichText::new(leading).monospace().color(MUTED));
                        ui.label(RichText::new(role).color(TEXT).size(12.0));
                        ui.end_row();
                    }
                });
        });

        ui.add_space(10.0);

        card(ui, "Font roles", |ui| {
            egui::Grid::new("info_font_roles")
                .striped(true)
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Font file").strong());
                    ui.label(RichText::new("Used for").strong());
                    ui.end_row();

                    for (font, description) in [
                        (
                            "ExuberanceF-Regular.ttf",
                            "Normal text, body text, and UI labels through $YG760, $YG760_12pt, and $YG330.",
                        ),
                        (
                            "MonarchaF-Regular.ttf",
                            "Titles and Damage Numbers through $YoonGasiIIM, $YoonGasiIIM_Small2pt, $ChronologyFont, and $eventDamageFont.",
                        ),
                    ] {
                        ui.label(RichText::new(font).monospace().color(ACCENT));
                        ui.label(RichText::new(description).color(TEXT).size(12.0));
                        ui.end_row();
                    }
                });
        });
    }

    fn game_card(&mut self, ui: &mut egui::Ui) {
        let mut profile_label = self.profile.label;
        let mut language_label = self.language.clone();

        card(ui, "Game and language", |ui| {
            ui.horizontal(|ui| {
                ui.label("Game profile:").on_hover_text(
                    "Choose the client that owns this font.lpk. Steam is the default.",
                );
                egui::ComboBox::from_id_salt("profile")
                    .selected_text(profile_label)
                    .show_ui(ui, |ui| {
                        for profile in PROFILES {
                            ui.selectable_value(&mut profile_label, profile.label, profile.label);
                        }
                    });

                ui.add_space(12.0);
                ui.label("Language:")
                    .on_hover_text("The font folder the game uses for your language.");
                egui::ComboBox::from_id_salt("language")
                    .selected_text(language_label.clone())
                    .show_ui(ui, |ui| {
                        for language in LANGUAGES {
                            ui.selectable_value(
                                &mut language_label,
                                language.label.to_string(),
                                language.label,
                            );
                        }
                    });
            });

            ui.add_space(6.0);
            status_row(
                ui,
                "Game folder",
                self.game_dir.as_deref(),
                "Not detected. Browse to the EFGame folder.",
            );
            ui.horizontal(|ui| {
                if ui.add(secondary_button("Detect")).clicked() {
                    self.detect_game();
                }
                if ui.add(secondary_button("Browse...")).clicked() {
                    self.pick_game_dir();
                }
            });
        });

        if profile_label != self.profile.label {
            self.profile = Profile::from_label(profile_label);
            if self.archive.is_some()
                && let Some(path) = self.archive_path.clone()
            {
                self.load_archive(path);
            }
        }
        if language_label != self.language {
            self.language = language_label;
        }
    }

    fn fonts_card(&mut self, ui: &mut egui::Ui) {
        let mut gained = None;
        let mut lost = None;
        let mut remove = None;
        let mut add_fonts = false;
        let mut import_fonts = false;
        let mut clear_fonts = false;

        card(ui, "Fonts", |ui| {
            if self.fonts.is_empty() {
                ui.label(RichText::new("No fonts added yet.").color(ORANGE));
            } else {
                egui::Grid::new("fonts_grid")
                    .striped(true)
                    .num_columns(3)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Font file").strong());
                        ui.label(RichText::new("Name in archive").strong());
                        ui.label("");
                        ui.end_row();

                        for (index, font) in self.fonts.iter_mut().enumerate() {
                            let missing = !font.path.is_file();
                            let path_text = font.path.display().to_string();
                            let color = if missing { RED } else { TEXT };
                            ui.add_sized(
                                [340.0, 24.0],
                                Label::new(
                                    RichText::new(file_name(&font.path))
                                        .monospace()
                                        .color(color),
                                )
                                .truncate(),
                            )
                            .on_hover_text(if missing {
                                format!("Missing: {path_text}")
                            } else {
                                path_text
                            });

                            let response = ui.add_sized(
                                [320.0, 24.0],
                                TextEdit::singleline(&mut font.name).desired_width(320.0),
                            );
                            if response.gained_focus() {
                                gained = Some((index, font.name.trim().to_string()));
                            }
                            if response.lost_focus() {
                                lost = Some(index);
                            }

                            if ui.small_button("Remove").clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(secondary_button("Add fonts...")).clicked() {
                    add_fonts = true;
                }
                let can_import = !self.busy && self.game_archive_path().is_some();
                if ui
                    .add_enabled(
                        can_import,
                        secondary_button("Import fonts from game..."),
                    )
                    .on_hover_text(
                        "Adds the fonts from the selected language folder in the game's original font.lpk.",
                    )
                    .clicked()
                {
                    import_fonts = true;
                }
                if !self.fonts.is_empty() && ui.add(secondary_button("Clear fonts")).clicked() {
                    clear_fonts = true;
                }
            });
        });

        if let Some((index, name)) = gained {
            self.rename_origin = Some((index, name));
        }
        if let Some(index) = lost
            && let Some((origin_index, old_name)) = self.rename_origin.take()
            && origin_index == index
            && let Some(font) = self.fonts.get(index)
        {
            let new_name = font.name.trim().to_string();
            if !new_name.is_empty() && new_name != old_name {
                for row in &mut self.fontmap_rows {
                    if row.file.trim() == old_name {
                        row.file = new_name.clone();
                    }
                }
            }
        }
        if add_fonts {
            self.pick_fonts();
        }
        if import_fonts {
            self.import_game_fonts();
        }
        if clear_fonts {
            self.fonts.clear();
            self.sync_fontmap_to_fonts();
        }
        if let Some(index) = remove
            && index < self.fonts.len()
        {
            self.fonts.remove(index);
            self.sync_fontmap_to_fonts();
        }
    }

    fn fontmap_card(&mut self, ui: &mut egui::Ui) {
        let font_names: Vec<String> = self
            .fonts
            .iter()
            .map(|font| font.name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        let mut remove = None;

        card(ui, "FontMap rows", |ui| {
            if self.fontmap_rows.is_empty() {
                ui.label(
                    RichText::new("No rows yet. Add a row or load the defaults from the game.")
                        .color(ORANGE),
                );
            } else {
                egui::Grid::new("fontmap_grid")
                    .striped(true)
                    .num_columns(5)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Key").strong())
                            .on_hover_text("Text style name used by the game.");
                        ui.label(RichText::new("Font").strong())
                            .on_hover_text("One of the fonts added above.");
                        ui.label(RichText::new("Size").strong());
                        ui.label(RichText::new("Leading").strong());
                        ui.label("");
                        ui.end_row();

                        for (index, row) in self.fontmap_rows.iter_mut().enumerate() {
                            ui.add_sized(
                                [340.0, 24.0],
                                TextEdit::singleline(&mut row.key).desired_width(340.0),
                            );
                            egui::ComboBox::from_id_salt(("fontmap_font", index))
                                .width(220.0)
                                .selected_text(if row.file.trim().is_empty() {
                                    "Select a font".to_string()
                                } else {
                                    row.file.clone()
                                })
                                .show_ui(ui, |ui| {
                                    for name in &font_names {
                                        ui.selectable_value(&mut row.file, name.clone(), name);
                                    }
                                });
                            ui.add_sized(
                                [120.0, 24.0],
                                TextEdit::singleline(&mut row.size).desired_width(120.0),
                            );
                            ui.add_sized(
                                [140.0, 24.0],
                                TextEdit::singleline(&mut row.leading).desired_width(140.0),
                            );
                            if ui.small_button("Remove").clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(secondary_button("Add row")).clicked() {
                    let file = font_names.first().cloned().unwrap_or_default();
                    self.fontmap_rows.push(FontItem {
                        key: "$NewKey".to_string(),
                        file,
                        ..Default::default()
                    });
                }
                if ui
                    .add_enabled(
                        !self.busy && self.game_archive_path().is_some(),
                        secondary_button("Load defaults from game"),
                    )
                    .clicked()
                {
                    self.import_fontmap();
                }
            });
        });

        if let Some(index) = remove
            && index < self.fontmap_rows.len()
        {
            self.fontmap_rows.remove(index);
        }
    }

    fn build_card(&mut self, ui: &mut egui::Ui) {
        let problem = self.package_problem();
        let output_text = self.output_path.as_ref().map_or_else(
            || "Not selected".to_string(),
            |path| path.display().to_string(),
        );
        let game_ready = self.has_game();
        let built = self.built_archive();
        let language = self.selected_language();
        let font_count = self.fonts.len();
        let row_count = self.fontmap_rows.len();

        card(ui, "Build and install", |ui| {
            ui.horizontal(|ui| {
                ui.label("Output:");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(secondary_button("Save as...")).clicked() {
                        self.pick_output();
                    }
                    ui.add(
                        Label::new(RichText::new(output_text.clone()).monospace().color(TEXT))
                            .truncate(),
                    )
                    .on_hover_text(output_text.clone());
                });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Package: 1 FontMap.xml + {font_count} font(s) in {} ({row_count} FontMap rows).",
                    language.folder
                ))
                .color(MUTED)
                .size(12.0),
            );

            if let Some(problem) = &problem {
                ui.add_space(6.0);
                ui.label(RichText::new(problem).color(ORANGE).size(12.0));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let can_build = problem.is_none() && !self.busy;
                let can_install = can_build && game_ready;
                if ui
                    .add_enabled(can_install, primary_button("Build and install"))
                    .clicked()
                {
                    self.build(true);
                }
                if ui
                    .add_enabled(can_build, secondary_button("Build only"))
                    .clicked()
                {
                    self.build(false);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let can_install_built =
                    built.as_ref().is_some_and(|path| path.is_file()) && game_ready && !self.busy;
                if ui
                    .add_enabled(can_install_built, secondary_button("Install built package"))
                    .clicked()
                {
                    self.install_last_built();
                }
                if ui
                    .add_enabled(
                        game_ready && !self.busy,
                        secondary_button("Restore original"),
                    )
                    .clicked()
                {
                    self.revert();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        game_ready && !self.busy,
                        secondary_button("Install existing font.lpk..."),
                    )
                    .on_hover_text(
                        "Select any font.lpk or font.lpk.bak file to install into the game folder.",
                    )
                    .clicked()
                {
                    self.pick_existing_install();
                }
            });

            if let Some(path) = &built {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("Built package: {}", path.display()))
                        .monospace()
                        .color(MUTED)
                        .size(11.0),
                );
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "The game's current font.lpk is backed up as font.lpk.bak before install. Build only never touches the game.",
                )
                .color(MUTED)
                .size(11.0),
            );
        });
    }

    fn extract_page(&mut self, ui: &mut egui::Ui) {
        if self.archive.is_none() {
            card(ui, "Open an archive", |ui| {
                ui.label(RichText::new("Read font.lpk directly.").color(MUTED));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let game_ready = self.game_archive_path().is_some();
                    if ui
                        .add_enabled(game_ready && !self.busy, primary_button("Use font.lpk from game directory"))
                        .on_hover_text(
                            "Opens font.lpk.bak if it exists, otherwise font.lpk, from the game folder.",
                        )
                        .clicked()
                    {
                        self.open_game_archive();
                    }
                    if ui.add(secondary_button("Browse...")).clicked() {
                        self.open_archive_dialog();
                    }
                    if !game_ready
                        && ui
                            .add_enabled(
                                !self.busy,
                                secondary_button("Find game folder"),
                            )
                            .clicked()
                    {
                        self.detect_game();
                        if self.game_archive_path().is_some() {
                            self.open_game_archive();
                        }
                    }
                });
                if self.game_archive_path().is_none() {
                    ui.label(
                        RichText::new(
                            "Use 'Find game folder' to detect the Steam install, or select the EFGame folder on the Fonts tab.",
                        )
                        .color(MUTED)
                        .size(11.0),
                    );
                }
            });
            return;
        }

        let Some(archive) = self.archive.clone() else {
            return;
        };
        let path_text = self
            .archive_path
            .as_ref()
            .map_or_else(String::new, |path| path.display().to_string());
        let entry_count = archive.entries.len();
        let total = archive
            .entries
            .iter()
            .map(|entry| entry.ensize as u64)
            .sum::<u64>();
        let mut profile_label = self.profile.label;

        card(ui, "Archive overview", |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(secondary_button("Extract all...")).clicked() {
                        self.extract_all();
                    }
                    if ui
                        .add_enabled(!self.busy, secondary_button("Reload"))
                        .clicked()
                        && let Some(path) = self.archive_path.clone()
                    {
                        self.load_archive(path);
                    }
                    if ui
                        .add_enabled(
                            !self.busy && self.game_archive_path().is_some(),
                            secondary_button("Use font.lpk from game directory"),
                        )
                        .clicked()
                    {
                        self.open_game_archive();
                    }
                    if ui
                        .add_enabled(!self.busy, secondary_button("Open..."))
                        .clicked()
                    {
                        self.open_archive_dialog();
                    }
                    ui.add(
                        Label::new(RichText::new(path_text.clone()).monospace().color(TEXT))
                            .truncate(),
                    )
                    .on_hover_text(path_text.clone());
                });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{entry_count} entries  |  {}  |  {:.1} MB",
                    self.profile.label,
                    total as f64 / 1_048_576.0
                ))
                .color(MUTED)
                .size(12.0),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Profile:").on_hover_text(
                    "The client that owns this archive. Change it if the table cannot be read.",
                );
                egui::ComboBox::from_id_salt("extract_profile")
                    .selected_text(profile_label)
                    .show_ui(ui, |ui| {
                        for profile in PROFILES {
                            ui.selectable_value(&mut profile_label, profile.label, profile.label);
                        }
                    });
            });
        });

        if profile_label != self.profile.label {
            self.profile = Profile::from_label(profile_label);
            if let Some(path) = self.archive_path.clone() {
                self.load_archive(path);
            }
        }

        ui.add_space(10.0);

        card(ui, "Contents", |ui| {
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .hint_text("Search by file name or folder")
                        .desired_width(300.0),
                );
            });
            ui.add_space(6.0);

            let filter = self.filter.to_ascii_lowercase();
            let mut shown = 0;
            egui::Grid::new("entries_grid")
                .striped(true)
                .num_columns(3)
                .spacing([14.0, 5.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("#").strong());
                    ui.label(RichText::new("Size").strong());
                    ui.label(RichText::new("Path").strong());
                    ui.end_row();

                    for (index, entry) in archive.entries.iter().enumerate() {
                        let path = entry.display_path();
                        if !filter.is_empty() && !path.to_ascii_lowercase().contains(&filter) {
                            continue;
                        }
                        if shown >= 500 {
                            break;
                        }
                        shown += 1;
                        ui.label(
                            RichText::new(format!("{:>2}", index + 1))
                                .monospace()
                                .color(MUTED),
                        );
                        ui.label(
                            RichText::new(format_bytes(entry.size as u64))
                                .monospace()
                                .color(MUTED),
                        );
                        ui.label(RichText::new(path).monospace().size(12.0));
                        ui.end_row();
                    }
                });

            let matching = archive
                .entries
                .iter()
                .filter(|entry| {
                    filter.is_empty() || entry.display_path().to_ascii_lowercase().contains(&filter)
                })
                .count();
            if matching > shown {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Showing {shown} of {matching} matching entries. Refine the filter to see the rest."
                    ))
                    .color(MUTED)
                    .size(11.0),
                );
            }
        });
    }
}

impl eframe::App for FontStudioApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_messages();
        self.header(root);
        self.sidebar(root);
        self.log_panel(root);

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(BG).inner_margin(Margin::symmetric(18, 16)))
            .show(root, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Fonts => self.fonts_page(ui),
                        Page::BuildInstall => self.build_install_page(ui),
                        Page::Extract => self.extract_page(ui),
                        Page::Info => self.info_page(ui),
                    });
            });

        if self.busy {
            root.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        settings::save(&self.settings());
    }
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.faint_bg_color = CARD;
    visuals.extreme_bg_color = INPUT;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(124, 92, 255, 110);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.window_corner_radius = CornerRadius::same(14);

    for widgets in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widgets.corner_radius = CornerRadius::same(8);
    }

    visuals.widgets.noninteractive.bg_fill = CARD;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_fill = INPUT;
    visuals.widgets.inactive.weak_bg_fill = INPUT;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_fill = HOVER;
    visuals.widgets.hovered.weak_bg_fill = HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 7.0);
        style.spacing.interact_size.y = 32.0;
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(25.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(15.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        );
    });
}

fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::NONE
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(title).size(16.0).strong());
            ui.add_space(10.0);
            add(ui)
        })
        .inner
}

fn nav_button(ui: &mut egui::Ui, page: &mut Page, target: Page, text: &str) {
    let selected = *page == target;
    let response = ui.add_sized(
        [ui.available_width(), 38.0],
        Button::new(RichText::new(text).color(if selected { Color32::WHITE } else { MUTED }))
            .fill(if selected {
                ACCENT
            } else {
                Color32::TRANSPARENT
            })
            .corner_radius(CornerRadius::same(8)),
    );
    if response.clicked() {
        *page = target;
    }
}

fn primary_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text).strong())
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 34.0))
}

fn secondary_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text))
        .fill(INPUT)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 30.0))
}

fn status_chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::NONE
        .fill(color.gamma_multiply(0.16))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(11.0).strong());
        });
}

fn status_row(ui: &mut egui::Ui, label: &str, path: Option<&Path>, missing: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [124.0, 20.0],
            Label::new(RichText::new(label).color(MUTED).size(12.0)),
        );
        match path {
            Some(path) => {
                ui.add(
                    Label::new(
                        RichText::new(path.display().to_string())
                            .monospace()
                            .size(12.0)
                            .color(TEXT),
                    )
                    .truncate(),
                )
                .on_hover_text(path.display().to_string());
            }
            None => {
                ui.label(RichText::new(missing).color(MUTED).size(12.0));
            }
        }
    });
}

fn default_rows(font_file: &str) -> Vec<FontItem> {
    let file = font_file.to_string();
    vec![
        FontItem {
            key: "$YG760".to_string(),
            file: file.clone(),
            ..Default::default()
        },
        FontItem {
            key: "$YG760_12pt".to_string(),
            file: file.clone(),
            size: "120".to_string(),
            ..Default::default()
        },
        FontItem {
            key: "$YG330".to_string(),
            file: file.clone(),
            ..Default::default()
        },
        FontItem {
            key: "$YoonGasiIIM".to_string(),
            file: file.clone(),
            leading: "-200".to_string(),
            ..Default::default()
        },
        FontItem {
            key: "$YoonGasiIIM_Small2pt".to_string(),
            file: file.clone(),
            size: "120".to_string(),
            leading: "-200".to_string(),
            ..Default::default()
        },
        FontItem {
            key: "$ChronologyFont".to_string(),
            file: file.clone(),
            leading: "-200".to_string(),
            ..Default::default()
        },
        FontItem {
            key: "$eventDamageFont".to_string(),
            file,
            ..Default::default()
        },
    ]
}

fn build_entries(
    fonts: &[FontSetting],
    rows: &[FontItem],
    folder: &str,
) -> Result<Vec<PackageEntry>> {
    fontmap::validate(rows).context("FontMap")?;

    let names: HashSet<String> = fonts
        .iter()
        .map(|font| font.name.trim().to_ascii_lowercase())
        .collect();
    for row in rows {
        if !names.contains(&row.file.trim().to_ascii_lowercase()) {
            bail!(
                "FontMap row '{}' points to '{}', which is not in the font list.",
                row.key.trim(),
                row.file.trim()
            );
        }
    }

    let mut entries = Vec::with_capacity(fonts.len() + 1);
    entries.push(PackageEntry {
        name: format!("{folder}\\FontMap.xml"),
        data: fontmap::build(rows).into_bytes(),
    });

    for font in fonts {
        let data = fs::read(&font.path)
            .with_context(|| format!("Could not read {}", font.path.display()))?;
        if !looks_like_font(&data) {
            bail!(
                "{} does not look like a TrueType or OpenType font.",
                font.name
            );
        }
        entries.push(PackageEntry {
            name: format!("{folder}\\{}", font.name.trim()),
            data,
        });
    }

    Ok(entries)
}

fn unique_font_name(path: &Path, fonts: &[FontSetting]) -> String {
    let base = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let base = if base.is_empty() {
        "font.ttf".to_string()
    } else {
        base
    };

    let existing: HashSet<String> = fonts
        .iter()
        .map(|font| font.name.trim().to_ascii_lowercase())
        .collect();
    if !existing.contains(&base.to_ascii_lowercase()) {
        return base;
    }

    let (stem, extension) = match base.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (base, String::new()),
    };

    let mut number = 2;
    loop {
        let candidate = format!("{stem}-{number}{extension}");
        if !existing.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
        number += 1;
    }
}

fn sync_rows_to_fonts(rows: &mut [FontItem], fonts: &[FontSetting]) {
    for row in rows {
        let file = row.file.trim();
        if let Some(font) = fonts
            .iter()
            .find(|font| font.name.trim().eq_ignore_ascii_case(file))
        {
            row.file = font.name.trim().to_string();
        } else {
            row.file.clear();
        }
    }
}

fn entry_folder(name: &str) -> &str {
    name.rsplit_once(['\\', '/'])
        .map_or("", |(folder, _)| folder)
}

fn entry_file_name(name: &str) -> &str {
    name.rsplit(['\\', '/']).next().unwrap_or(name)
}

fn is_font_file(name: &str) -> bool {
    let extension = entry_file_name(name)
        .rsplit_once('.')
        .map_or("", |(_, extension)| extension);
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "ttf" | "otf" | "ttc"
    )
}

fn imported_fonts_dir(language: &str) -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("LOAFontStudio")
        .join("imported")
        .join(language)
}

fn default_output(game_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = game_dir {
        return dir.join("font_mod.lpk");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("LOAFontStudio")
        .join("font_mod.lpk")
}

fn valid_font_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['\\', '/', ':'])
}

fn looks_like_font(data: &[u8]) -> bool {
    let magic = data.get(..4).unwrap_or(&[]);
    magic == b"\x00\x01\x00\x00" || magic == b"OTTO" || magic == b"true" || magic == b"ttcf"
}

fn font_magic(path: &Path) -> bool {
    use std::io::Read;

    let mut magic = [0u8; 4];
    fs::File::open(path)
        .ok()
        .and_then(|mut file| file.read_exact(&mut magic).ok())
        .is_some_and(|_| looks_like_font(&magic))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => {
            let a_parent = a.parent().and_then(|path| path.canonicalize().ok());
            let b_parent = b.parent().and_then(|path| path.canonicalize().ok());
            a_parent.is_some() && a_parent == b_parent && a.file_name() == b.file_name()
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
#[path = "../tests/unit/app.rs"]
mod tests;
