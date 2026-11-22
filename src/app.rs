use crate::buffer::{char_to_byte_idx, expand_tabs_for_display, TextBuffer};
use crate::dialog::{spawn_open_dialog, spawn_save_dialog, DialogResult};
use crate::filer::{
    copy_entry, create_dir, create_file, delete_entry, format_size, move_entry, rename_entry,
    ActivePane, FileEntry, FilerState,
};
use crate::fonts::configure_japanese_fonts;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Event, FontId, ImeEvent, Key, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

// --- PC-98 / VZ Editor Color Palette ---
const BG_NAVY: Color32 = Color32::from_rgb(8, 24, 56); // #081838
const BG_ACTIVE_LINE: Color32 = Color32::from_rgb(14, 38, 84);
const BG_GUTTER: Color32 = Color32::from_rgb(6, 19, 46);
const BG_TAB_BAR: Color32 = Color32::from_rgb(9, 28, 66);
const BG_TAB_ACTIVE: Color32 = Color32::from_rgb(24, 66, 134);
const BG_TAB_INACTIVE: Color32 = Color32::from_rgb(12, 34, 78);

const BORDER_BLUE: Color32 = Color32::from_rgb(42, 98, 168);
const BORDER_ACTIVE_PANE: Color32 = Color32::from_rgb(90, 212, 255);
const GUTTER_SEP: Color32 = Color32::from_rgb(26, 62, 118);

const STATUS_BG: Color32 = Color32::from_rgb(15, 42, 92);
const STATUS_BADGE_BG: Color32 = Color32::from_rgb(64, 210, 255);
const STATUS_BADGE_FILER_BG: Color32 = Color32::from_rgb(255, 206, 86);
const STATUS_BADGE_FG: Color32 = Color32::from_rgb(6, 20, 48);

const FKEY_BAR_BG: Color32 = Color32::from_rgb(10, 28, 64);
const FKEY_BTN_BG: Color32 = Color32::from_rgb(18, 48, 102);
const FKEY_BTN_HOVER: Color32 = Color32::from_rgb(30, 74, 148);
const FKEY_NUM_BG: Color32 = Color32::from_rgb(46, 140, 208);
const FKEY_NUM_FILER_BG: Color32 = Color32::from_rgb(36, 138, 168);
const FKEY_NUM_FG: Color32 = Color32::from_rgb(240, 250, 255);

const FILER_HDR_ACTIVE: Color32 = Color32::from_rgb(18, 56, 114);
const FILER_HDR_INACTIVE: Color32 = Color32::from_rgb(11, 32, 72);
const FILER_COL_HDR: Color32 = Color32::from_rgb(9, 26, 60);
const FILER_ROW_CURSOR_ACTIVE: Color32 = Color32::from_rgb(28, 82, 156);
const FILER_ROW_CURSOR_INACTIVE: Color32 = Color32::from_rgb(16, 44, 92);
const FILER_ROW_MARKED: Color32 = Color32::from_rgb(62, 54, 18);
const FILER_ROW_MARKED_CURSOR: Color32 = Color32::from_rgb(88, 76, 24);

const TEXT_WHITE: Color32 = Color32::from_rgb(234, 244, 255);
const TEXT_CYAN: Color32 = Color32::from_rgb(112, 232, 255);
const TEXT_DIM_CYAN: Color32 = Color32::from_rgb(88, 146, 204);
const TEXT_CR_MARK: Color32 = Color32::from_rgb(56, 110, 168);
const TEXT_EOF: Color32 = Color32::from_rgb(76, 178, 224);
const TEXT_AMBER: Color32 = Color32::from_rgb(255, 220, 96);

const CURSOR_FILL: Color32 = Color32::from_rgba_premultiplied(80, 220, 255, 110);
const CURSOR_BAR: Color32 = Color32::from_rgb(120, 244, 255);
const PREEDIT_BG: Color32 = Color32::from_rgb(28, 72, 134);
const PREEDIT_FG: Color32 = Color32::from_rgb(255, 246, 160);
const PREEDIT_UNDERLINE: Color32 = Color32::from_rgb(112, 236, 255);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Editor,
    Filer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixKey {
    None,
    CtrlQ,
    CtrlK,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorAction {
    ToggleHelp,
    NewBuffer,
    OpenFile,
    MoveLineStart,
    MoveLineEnd,
    DeleteLine,
    MoveWordLeft,
    MoveWordRight,
    SwitchToFiler,
    SaveFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilerAction {
    ToggleHelp,
    Rename,
    NewFile,
    Copy,
    Move,
    Delete,
    NewDir,
    CycleSort,
    BackToEditor,
    Quit,
}

#[derive(Debug, Clone)]
struct InlinePathPrompt {
    is_save: bool,
    path_input: String,
    just_opened: bool,
}

#[derive(Debug, Clone)]
enum FilerModal {
    Rename {
        src: PathBuf,
        input: String,
        just_opened: bool,
    },
    NewFile {
        dir: PathBuf,
        input: String,
        just_opened: bool,
    },
    NewDir {
        dir: PathBuf,
        input: String,
        just_opened: bool,
    },
    ConfirmCopyOrMove {
        is_move: bool,
        targets: Vec<FileEntry>,
        dst_dir: PathBuf,
    },
    ConfirmDelete {
        targets: Vec<FileEntry>,
    },
    ConfirmQuit,
}

struct BufferSlot {
    buffer: TextBuffer,
    scroll_x: f32,
    scroll_y: f32,
}

impl BufferSlot {
    fn new(buffer: TextBuffer) -> Self {
        Self {
            buffer,
            scroll_x: 0.0,
            scroll_y: 0.0,
        }
    }

    fn is_untouched_untitled(&self) -> bool {
        self.buffer.file_path.is_none()
            && !self.buffer.dirty
            && self.buffer.lines.len() == 1
            && self.buffer.lines[0].is_empty()
    }
}

pub struct LzEditorApp {
    mode: AppMode,
    buffers: Vec<BufferSlot>,
    active_buf: usize,
    filer: FilerState,

    prefix: PrefixKey,
    status_message: String,
    ime_preedit: String,
    ime_active: bool,

    // Rendering / scrolling state
    font_size: f32,
    line_height: f32,
    need_scroll_to_cursor: bool,
    last_visible_lines: usize,
    last_filer_visible_rows: usize,

    // UI overlays & async native dialogs
    show_help: bool,
    pending_dialog: Option<Receiver<DialogResult>>,
    inline_prompt: Option<InlinePathPrompt>,
    filer_modal: Option<FilerModal>,
}

impl LzEditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        configure_japanese_fonts(&cc.egui_ctx);
        Self::configure_visuals(&cc.egui_ctx);

        let mut status_message =
            String::from("VZ互換: ^E/^S/^D/^X移動  ^Wバッファ切替  ESCファイラー  F1ヘルプ");
        let mut initial_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        let initial_buffer = if let Some(path) = initial_file {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && parent.exists() {
                    initial_dir = parent.to_path_buf();
                }
            }
            match TextBuffer::from_file(&path) {
                Ok(buf) => {
                    status_message = format!("読み込みました: {}", path.display());
                    buf
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        let mut buf = TextBuffer::new();
                        buf.file_path = Some(path.clone());
                        status_message = format!("新規ファイル: {}", path.display());
                        buf
                    } else {
                        status_message = format!("読み込みエラー: {e}");
                        TextBuffer::new()
                    }
                }
            }
        } else {
            TextBuffer::new()
        };

        Self {
            mode: AppMode::Editor,
            buffers: vec![BufferSlot::new(initial_buffer)],
            active_buf: 0,
            filer: FilerState::new(initial_dir),
            prefix: PrefixKey::None,
            status_message,
            ime_preedit: String::new(),
            ime_active: false,
            font_size: 16.0,
            line_height: 23.0,
            need_scroll_to_cursor: false,
            last_visible_lines: 24,
            last_filer_visible_rows: 20,
            show_help: false,
            pending_dialog: None,
            inline_prompt: None,
            filer_modal: None,
        }
    }

    fn configure_visuals(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG_NAVY;
        visuals.window_fill = STATUS_BG;
        visuals.override_text_color = Some(TEXT_WHITE);
        visuals.widgets.noninteractive.bg_fill = BG_NAVY;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_WHITE);
        ctx.set_visuals(visuals);
    }

    fn cur_slot(&self) -> &BufferSlot {
        &self.buffers[self.active_buf]
    }

    fn cur_slot_mut(&mut self) -> &mut BufferSlot {
        &mut self.buffers[self.active_buf]
    }

    fn cur_buf(&self) -> &TextBuffer {
        &self.buffers[self.active_buf].buffer
    }

    fn cur_buf_mut(&mut self) -> &mut TextBuffer {
        &mut self.buffers[self.active_buf].buffer
    }

    fn has_any_dirty_buffer(&self) -> bool {
        self.buffers.iter().any(|s| s.buffer.dirty)
    }

    // --- Mode Switching ---

    fn enter_filer(&mut self) {
        self.prefix = PrefixKey::None;
        self.ime_preedit.clear();
        self.show_help = false;
        self.filer.refresh_both();
        self.mode = AppMode::Filer;
        self.status_message = "2画面ファイラー (Tab/←→:窓切替  Space:マーク  Enter:開く  ESC:エディタ戻)"
            .to_string();
    }

    fn enter_editor(&mut self) {
        self.prefix = PrefixKey::None;
        self.show_help = false;
        self.filer_modal = None;
        self.mode = AppMode::Editor;
        self.need_scroll_to_cursor = true;
        let name = self.cur_buf().display_name();
        self.status_message = format!(
            "エディタ [{}/{}]: {}",
            self.active_buf + 1,
            self.buffers.len(),
            name
        );
    }

    // --- Multi-Buffer Management ---

    fn open_file_in_buffer(&mut self, path: PathBuf) {
        let canon_target = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());

        // If already open in an existing buffer, switch directly to it.
        if let Some(existing_idx) = self.buffers.iter().position(|slot| {
            slot.buffer.file_path.as_ref().is_some_and(|p| {
                let canon_p = fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                canon_p == canon_target
            })
        }) {
            self.active_buf = existing_idx;
            self.mode = AppMode::Editor;
            self.need_scroll_to_cursor = true;
            let name = self.cur_buf().display_name();
            self.status_message = format!(
                "既存バッファへ切替 [{}/{}]: {}",
                self.active_buf + 1,
                self.buffers.len(),
                name
            );
            return;
        }

        match TextBuffer::from_file(&path) {
            Ok(buf) => {
                let display_path = path.display().to_string();
                if self.buffers.len() == 1 && self.buffers[0].is_untouched_untitled() {
                    self.buffers[0] = BufferSlot::new(buf);
                    self.active_buf = 0;
                } else {
                    self.buffers.push(BufferSlot::new(buf));
                    self.active_buf = self.buffers.len() - 1;
                }
                self.mode = AppMode::Editor;
                self.need_scroll_to_cursor = true;
                self.status_message = format!(
                    "開きました [{}/{}]: {}",
                    self.active_buf + 1,
                    self.buffers.len(),
                    display_path
                );
            }
            Err(e) => {
                self.status_message = format!("読み込みエラー: {e}");
            }
        }
    }

    fn new_empty_buffer(&mut self) {
        self.buffers.push(BufferSlot::new(TextBuffer::new()));
        self.active_buf = self.buffers.len() - 1;
        self.mode = AppMode::Editor;
        self.need_scroll_to_cursor = true;
        self.status_message = format!(
            "新規バッファ作成 [{}/{}]",
            self.active_buf + 1,
            self.buffers.len()
        );
    }

    fn switch_buffer_step(&mut self, delta: isize) {
        let count = self.buffers.len();
        if count <= 1 {
            self.status_message =
                "開いているバッファは1件です（ESCファイラーから複数ファイルを開けます）"
                    .to_string();
            return;
        }
        let next = ((self.active_buf as isize + delta).rem_euclid(count as isize)) as usize;
        self.active_buf = next;
        self.prefix = PrefixKey::None;
        self.ime_preedit.clear();
        self.need_scroll_to_cursor = true;
        let name = self.cur_buf().display_name();
        self.status_message = format!("バッファ切替 [{}/{}]: {}", next + 1, count, name);
    }

    fn switch_buffer_to(&mut self, idx: usize) {
        if idx < self.buffers.len() {
            self.active_buf = idx;
            self.prefix = PrefixKey::None;
            self.ime_preedit.clear();
            self.need_scroll_to_cursor = true;
            let name = self.cur_buf().display_name();
            self.status_message =
                format!("バッファ切替 [{}/{}]: {}", idx + 1, self.buffers.len(), name);
        }
    }

    fn update_open_buffer_path(&mut self, old_path: &Path, new_path: &Path) {
        let canon_old = fs::canonicalize(old_path).unwrap_or_else(|_| old_path.to_path_buf());
        for slot in &mut self.buffers {
            if let Some(p) = &slot.buffer.file_path {
                let canon_p = fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                if canon_p == canon_old || p == old_path {
                    slot.buffer.file_path = Some(new_path.to_path_buf());
                }
            }
        }
    }

    // --- Native Dialogs (Editor) ---

    fn poll_dialog(&mut self) {
        let result = match &self.pending_dialog {
            Some(rx) => match rx.try_recv() {
                Ok(res) => Some(res),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending_dialog = None;
                    None
                }
            },
            None => None,
        };

        if let Some(res) = result {
            self.pending_dialog = None;
            match res {
                DialogResult::SavePath(Some(path)) => self.perform_save(path),
                DialogResult::SavePath(None) => {
                    self.status_message = "保存をキャンセルしました".to_string();
                }
                DialogResult::OpenPath(Some(path)) => self.open_file_in_buffer(path),
                DialogResult::OpenPath(None) => {
                    self.status_message = "ファイルを開くのをキャンセルしました".to_string();
                }
                DialogResult::Unavailable { is_save } => {
                    let initial = self
                        .cur_buf()
                        .file_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| {
                            std::env::current_dir()
                                .map(|d| d.join("untitled.txt").display().to_string())
                                .unwrap_or_else(|_| "untitled.txt".to_string())
                        });
                    self.inline_prompt = Some(InlinePathPrompt {
                        is_save,
                        path_input: initial,
                        just_opened: true,
                    });
                }
            }
        }
    }

    fn perform_save(&mut self, path: PathBuf) {
        match self.cur_buf_mut().save_to_path(&path) {
            Ok(()) => {
                self.status_message = format!("保存しました: {}", path.display());
            }
            Err(e) => {
                self.status_message = format!("保存エラー: {e}");
            }
        }
    }

    fn trigger_save(&mut self) {
        if let Some(path) = self.cur_buf().file_path.clone() {
            self.perform_save(path);
        } else if self.pending_dialog.is_none() {
            self.status_message = "保存先を選択中...".to_string();
            self.pending_dialog = Some(spawn_save_dialog(None));
        }
    }

    fn trigger_open(&mut self) {
        if self.pending_dialog.is_none() {
            self.status_message = "開くファイルを選択中...".to_string();
            let dir = self
                .cur_buf()
                .file_path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf());
            self.pending_dialog = Some(spawn_open_dialog(dir.as_deref()));
        }
    }

    // --- Editor Actions ---

    fn execute_editor_action(&mut self, action: EditorAction) {
        self.prefix = PrefixKey::None;
        match action {
            EditorAction::ToggleHelp => {
                self.show_help = !self.show_help;
            }
            EditorAction::NewBuffer => {
                self.new_empty_buffer();
            }
            EditorAction::OpenFile => {
                self.trigger_open();
            }
            EditorAction::MoveLineStart => {
                self.cur_buf_mut().move_line_start();
                self.need_scroll_to_cursor = true;
            }
            EditorAction::MoveLineEnd => {
                self.cur_buf_mut().move_line_end();
                self.need_scroll_to_cursor = true;
            }
            EditorAction::DeleteLine => {
                self.cur_buf_mut().delete_current_line();
                self.need_scroll_to_cursor = true;
                self.status_message = "現在行を削除しました (^Y)".to_string();
            }
            EditorAction::MoveWordLeft => {
                self.cur_buf_mut().move_word_left();
                self.need_scroll_to_cursor = true;
            }
            EditorAction::MoveWordRight => {
                self.cur_buf_mut().move_word_right();
                self.need_scroll_to_cursor = true;
            }
            EditorAction::SwitchToFiler => {
                self.enter_filer();
            }
            EditorAction::SaveFile => {
                self.trigger_save();
            }
        }
    }

    // --- Filer Actions ---

    fn filer_open_selected(&mut self) {
        let Some(entry) = self.filer.active_pane().selected_entry().cloned() else {
            return;
        };
        if entry.is_parent {
            if let Err(e) = self.filer.active_pane_mut().go_parent() {
                self.status_message = format!("移動エラー: {e}");
            }
        } else if entry.is_dir {
            if let Err(e) = self.filer.active_pane_mut().enter_dir(entry.path) {
                self.status_message = format!("ディレクトリを開けません: {e}");
            }
        } else {
            self.open_file_in_buffer(entry.path);
        }
    }

    fn execute_filer_action(&mut self, ctx: &egui::Context, action: FilerAction) {
        match action {
            FilerAction::ToggleHelp => {
                self.show_help = !self.show_help;
            }
            FilerAction::Rename => {
                if let Some(entry) = self.filer.active_pane().selected_entry().cloned() {
                    if entry.is_parent {
                        self.status_message = "親ディレクトリ (..) はリネームできません".to_string();
                    } else {
                        self.filer_modal = Some(FilerModal::Rename {
                            src: entry.path,
                            input: entry.name,
                            just_opened: true,
                        });
                    }
                }
            }
            FilerAction::NewFile => {
                let dir = self.filer.active_pane().current_dir.clone();
                self.filer_modal = Some(FilerModal::NewFile {
                    dir,
                    input: String::new(),
                    just_opened: true,
                });
            }
            FilerAction::Copy => {
                let targets = self.filer.active_pane().target_entries();
                if targets.is_empty() {
                    self.status_message = "コピー対象を選択してください".to_string();
                } else {
                    let dst_dir = self.filer.inactive_pane().current_dir.clone();
                    self.filer_modal = Some(FilerModal::ConfirmCopyOrMove {
                        is_move: false,
                        targets,
                        dst_dir,
                    });
                }
            }
            FilerAction::Move => {
                let targets = self.filer.active_pane().target_entries();
                if targets.is_empty() {
                    self.status_message = "移動対象を選択してください".to_string();
                } else {
                    let dst_dir = self.filer.inactive_pane().current_dir.clone();
                    self.filer_modal = Some(FilerModal::ConfirmCopyOrMove {
                        is_move: true,
                        targets,
                        dst_dir,
                    });
                }
            }
            FilerAction::Delete => {
                let targets = self.filer.active_pane().target_entries();
                if targets.is_empty() {
                    self.status_message = "削除対象を選択してください".to_string();
                } else {
                    self.filer_modal = Some(FilerModal::ConfirmDelete { targets });
                }
            }
            FilerAction::NewDir => {
                let dir = self.filer.active_pane().current_dir.clone();
                self.filer_modal = Some(FilerModal::NewDir {
                    dir,
                    input: String::new(),
                    just_opened: true,
                });
            }
            FilerAction::CycleSort => {
                let mode = self.filer.active_pane_mut().cycle_sort();
                self.status_message = format!("ソート順を変更しました: {}", mode.label());
            }
            FilerAction::BackToEditor => {
                self.enter_editor();
            }
            FilerAction::Quit => {
                if self.has_any_dirty_buffer() {
                    self.filer_modal = Some(FilerModal::ConfirmQuit);
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    // --- Event Dispatch ---

    fn handle_events(&mut self, ctx: &egui::Context) {
        if self.inline_prompt.is_some() || self.filer_modal.is_some() {
            return;
        }
        match self.mode {
            AppMode::Editor => self.handle_editor_events(ctx),
            AppMode::Filer => self.handle_filer_events(ctx),
        }
    }

    fn handle_editor_events(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        let modifiers_now = ctx.input(|i| i.modifiers);
        let mut suppress_text_this_frame = false;

        for event in events {
            match event {
                Event::Ime(ime_ev) => match ime_ev {
                    ImeEvent::Enabled => {
                        self.ime_active = true;
                    }
                    ImeEvent::Preedit(text) => {
                        self.ime_preedit = text;
                        self.need_scroll_to_cursor = true;
                    }
                    ImeEvent::Commit(text) => {
                        self.ime_preedit.clear();
                        if !text.is_empty() {
                            self.prefix = PrefixKey::None;
                            self.cur_buf_mut().insert_text(&text);
                            self.need_scroll_to_cursor = true;
                        }
                    }
                    ImeEvent::Disabled => {
                        self.ime_active = false;
                        self.ime_preedit.clear();
                    }
                },

                // egui_winit translates Ctrl+X to Event::Cut -> VZ diamond cursor DOWN!
                Event::Cut => {
                    if !self.ime_preedit.is_empty() {
                        continue;
                    }
                    suppress_text_this_frame = true;
                    if self.prefix == PrefixKey::None {
                        self.cur_buf_mut().move_down();
                        self.need_scroll_to_cursor = true;
                    } else {
                        self.prefix = PrefixKey::None;
                        self.status_message.clear();
                    }
                }

                // egui_winit translates Ctrl+C to Event::Copy -> supports Ctrl+Q+C (doc end).
                Event::Copy => {
                    if !self.ime_preedit.is_empty() {
                        continue;
                    }
                    suppress_text_this_frame = true;
                    if self.prefix == PrefixKey::CtrlQ {
                        self.prefix = PrefixKey::None;
                        self.cur_buf_mut().move_doc_end();
                        self.need_scroll_to_cursor = true;
                        self.status_message = "ファイル末尾へ移動 (^Q^C)".to_string();
                    } else {
                        self.prefix = PrefixKey::None;
                    }
                }

                Event::Paste(text) => {
                    if !self.ime_preedit.is_empty() {
                        continue;
                    }
                    suppress_text_this_frame = true;
                    self.prefix = PrefixKey::None;
                    if !text.is_empty() {
                        self.cur_buf_mut().insert_text(&text);
                        self.need_scroll_to_cursor = true;
                    }
                }

                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if !self.ime_preedit.is_empty() {
                        continue;
                    }

                    let ctrl = modifiers.ctrl || modifiers.command;

                    // 1. Two-stroke VZ prefix keys (^Q, ^K)
                    match self.prefix {
                        PrefixKey::CtrlQ => {
                            suppress_text_this_frame = true;
                            self.prefix = PrefixKey::None;
                            match key {
                                Key::S => {
                                    self.cur_buf_mut().move_line_start();
                                    self.need_scroll_to_cursor = true;
                                    self.status_message = "行頭へ移動 (^Q^S)".to_string();
                                }
                                Key::D => {
                                    self.cur_buf_mut().move_line_end();
                                    self.need_scroll_to_cursor = true;
                                    self.status_message = "行末へ移動 (^Q^D)".to_string();
                                }
                                Key::R => {
                                    self.cur_buf_mut().move_doc_start();
                                    self.need_scroll_to_cursor = true;
                                    self.status_message = "ファイル先頭へ移動 (^Q^R)".to_string();
                                }
                                Key::C => {
                                    self.cur_buf_mut().move_doc_end();
                                    self.need_scroll_to_cursor = true;
                                    self.status_message = "ファイル末尾へ移動 (^Q^C)".to_string();
                                }
                                Key::Escape => {
                                    self.status_message = "コマンドをキャンセルしました".to_string();
                                }
                                _ => {
                                    self.status_message = format!("未定義の ^Q コマンド: {key:?}");
                                }
                            }
                            continue;
                        }
                        PrefixKey::CtrlK => {
                            suppress_text_this_frame = true;
                            self.prefix = PrefixKey::None;
                            match key {
                                Key::D => {
                                    self.trigger_save();
                                }
                                Key::Q => {
                                    self.enter_filer();
                                    return;
                                }
                                Key::Escape => {
                                    self.status_message = "コマンドをキャンセルしました".to_string();
                                }
                                _ => {
                                    self.status_message = format!("未定義の ^K コマンド: {key:?}");
                                }
                            }
                            continue;
                        }
                        PrefixKey::None => {}
                    }

                    // 2. Function keys F1..F10
                    let fkey_action = match key {
                        Key::F1 => Some(EditorAction::ToggleHelp),
                        Key::F2 => Some(EditorAction::NewBuffer),
                        Key::F3 => Some(EditorAction::OpenFile),
                        Key::F4 => Some(EditorAction::MoveLineStart),
                        Key::F5 => Some(EditorAction::MoveLineEnd),
                        Key::F6 => Some(EditorAction::DeleteLine),
                        Key::F7 => Some(EditorAction::MoveWordLeft),
                        Key::F8 => Some(EditorAction::MoveWordRight),
                        Key::F9 => Some(EditorAction::SwitchToFiler),
                        Key::F10 => Some(EditorAction::SaveFile),
                        _ => None,
                    };
                    if let Some(action) = fkey_action {
                        suppress_text_this_frame = true;
                        self.execute_editor_action(action);
                        if self.mode == AppMode::Filer {
                            return;
                        }
                        continue;
                    }

                    // 3. ESC: instant switch to 2-pane Filer! (Closes help first if open)
                    if key == Key::Escape {
                        suppress_text_this_frame = true;
                        if self.show_help {
                            self.show_help = false;
                        } else {
                            self.enter_filer();
                            return;
                        }
                        continue;
                    }

                    // 4. VZ Ctrl-based bindings
                    if ctrl && !modifiers.alt {
                        let handled = match key {
                            // Diamond cursor: Ctrl+E / S / D / X
                            Key::E => {
                                self.cur_buf_mut().move_up();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            Key::S => {
                                self.cur_buf_mut().move_left();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            Key::D => {
                                self.cur_buf_mut().move_right();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            Key::X => {
                                self.cur_buf_mut().move_down();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            // Word left / right: Ctrl+A / Ctrl+F
                            Key::A => {
                                self.cur_buf_mut().move_word_left();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            Key::F => {
                                self.cur_buf_mut().move_word_right();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            // Multi-buffer switch (裏画面切替): Ctrl+W / Ctrl+Shift+W
                            Key::W => {
                                if modifiers.shift {
                                    self.switch_buffer_step(-1);
                                } else {
                                    self.switch_buffer_step(1);
                                }
                                true
                            }
                            // Delete current line: Ctrl+Y
                            Key::Y => {
                                self.cur_buf_mut().delete_current_line();
                                self.need_scroll_to_cursor = true;
                                self.status_message = "現在行を削除しました (^Y)".to_string();
                                true
                            }
                            // Delete char at cursor: Ctrl+G
                            Key::G => {
                                self.cur_buf_mut().delete_char_at_cursor();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            // BackSpace: Ctrl+H
                            Key::H => {
                                self.cur_buf_mut().backspace();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            // Enter: Ctrl+M
                            Key::M => {
                                self.cur_buf_mut().insert_newline();
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            // Prefix keys: Ctrl+Q, Ctrl+K
                            Key::Q => {
                                self.prefix = PrefixKey::CtrlQ;
                                self.status_message =
                                    "^Q  [S:行頭  D:行末  R:先頭  C:末尾  ESC:取消]".to_string();
                                true
                            }
                            Key::K => {
                                self.prefix = PrefixKey::CtrlK;
                                self.status_message =
                                    "^K  [D:保存  Q:ファイラー  ESC:取消]".to_string();
                                true
                            }
                            // Page up: Ctrl+R
                            Key::R => {
                                let lines = self.last_visible_lines;
                                self.cur_buf_mut().move_page_up(lines);
                                self.need_scroll_to_cursor = true;
                                true
                            }
                            _ => false,
                        };
                        if handled {
                            suppress_text_this_frame = true;
                            continue;
                        }
                    }

                    // 5. Standard navigation & editing keys
                    if !ctrl && !modifiers.alt {
                        match key {
                            Key::ArrowUp => {
                                self.cur_buf_mut().move_up();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::ArrowDown => {
                                self.cur_buf_mut().move_down();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::ArrowLeft => {
                                self.cur_buf_mut().move_left();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::ArrowRight => {
                                self.cur_buf_mut().move_right();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::Home => {
                                self.cur_buf_mut().move_line_start();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::End => {
                                self.cur_buf_mut().move_line_end();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::PageUp => {
                                let lines = self.last_visible_lines;
                                self.cur_buf_mut().move_page_up(lines);
                                self.need_scroll_to_cursor = true;
                            }
                            Key::PageDown => {
                                let lines = self.last_visible_lines;
                                self.cur_buf_mut().move_page_down(lines);
                                self.need_scroll_to_cursor = true;
                            }
                            Key::Enter => {
                                self.cur_buf_mut().insert_newline();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::Backspace => {
                                self.cur_buf_mut().backspace();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::Delete => {
                                self.cur_buf_mut().delete_char_at_cursor();
                                self.need_scroll_to_cursor = true;
                            }
                            Key::Tab => {
                                self.cur_buf_mut().insert_text("\t");
                                self.need_scroll_to_cursor = true;
                            }
                            _ => {}
                        }
                    }
                }

                Event::Text(text) => {
                    if suppress_text_this_frame
                        || !self.ime_preedit.is_empty()
                        || self.prefix != PrefixKey::None
                        || modifiers_now.ctrl
                        || modifiers_now.command
                        || modifiers_now.alt
                    {
                        continue;
                    }
                    let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !filtered.is_empty() {
                        self.cur_buf_mut().insert_text(&filtered);
                        self.need_scroll_to_cursor = true;
                    }
                }

                _ => {}
            }
        }
    }

    fn handle_filer_events(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());

        for event in events {
            match event {
                // Ctrl+X is converted to Event::Cut by egui_winit -> Filer cursor DOWN!
                Event::Cut => {
                    self.filer.active_pane_mut().move_down();
                }

                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let ctrl = modifiers.ctrl || modifiers.command;

                    // Function keys F1..F10
                    let fkey_action = match key {
                        Key::F1 => Some(FilerAction::ToggleHelp),
                        Key::F2 => Some(FilerAction::Rename),
                        Key::F3 => Some(FilerAction::NewFile),
                        Key::F4 => Some(FilerAction::Copy),
                        Key::F5 => Some(FilerAction::Move),
                        Key::F6 => Some(FilerAction::Delete),
                        Key::F7 => Some(FilerAction::NewDir),
                        Key::F8 => Some(FilerAction::CycleSort),
                        Key::F9 => Some(FilerAction::BackToEditor),
                        Key::F10 => Some(FilerAction::Quit),
                        _ => None,
                    };
                    if let Some(action) = fkey_action {
                        self.execute_filer_action(ctx, action);
                        if self.mode == AppMode::Editor || self.filer_modal.is_some() {
                            return;
                        }
                        continue;
                    }

                    // ESC: close help if open, else return immediately to Editor!
                    if key == Key::Escape {
                        if self.show_help {
                            self.show_help = false;
                        } else {
                            self.enter_editor();
                            return;
                        }
                        continue;
                    }

                    if ctrl && !modifiers.alt {
                        match key {
                            Key::E => self.filer.active_pane_mut().move_up(),
                            Key::X => self.filer.active_pane_mut().move_down(),
                            Key::S => {
                                self.filer.active = ActivePane::Left;
                            }
                            Key::D => {
                                self.filer.active = ActivePane::Right;
                            }
                            Key::H => {
                                if let Err(e) = self.filer.active_pane_mut().go_parent() {
                                    self.status_message = format!("移動エラー: {e}");
                                }
                            }
                            Key::M => {
                                self.filer_open_selected();
                                if self.mode == AppMode::Editor {
                                    return;
                                }
                            }
                            Key::R => {
                                let rows = self.last_filer_visible_rows;
                                self.filer.active_pane_mut().move_page_up(rows);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if !ctrl && !modifiers.alt {
                        match key {
                            Key::Tab => {
                                self.filer.active = self.filer.active.toggled();
                            }
                            Key::ArrowLeft => {
                                self.filer.active = ActivePane::Left;
                            }
                            Key::ArrowRight => {
                                self.filer.active = ActivePane::Right;
                            }
                            Key::ArrowUp => {
                                self.filer.active_pane_mut().move_up();
                            }
                            Key::ArrowDown => {
                                self.filer.active_pane_mut().move_down();
                            }
                            Key::PageUp => {
                                let rows = self.last_filer_visible_rows;
                                self.filer.active_pane_mut().move_page_up(rows);
                            }
                            Key::PageDown => {
                                let rows = self.last_filer_visible_rows;
                                self.filer.active_pane_mut().move_page_down(rows);
                            }
                            Key::Home => {
                                self.filer.active_pane_mut().move_top();
                            }
                            Key::End => {
                                self.filer.active_pane_mut().move_bottom();
                            }
                            Key::Space => {
                                self.filer.active_pane_mut().toggle_mark_and_advance();
                            }
                            Key::Backspace => {
                                if let Err(e) = self.filer.active_pane_mut().go_parent() {
                                    self.status_message = format!("移動エラー: {e}");
                                }
                            }
                            Key::Enter => {
                                self.filer_open_selected();
                                if self.mode == AppMode::Editor {
                                    return;
                                }
                            }
                            _ => {}
                        }
                    }
                }

                _ => {}
            }
        }
    }

    // --- Status Bar & Multi-Buffer Strip ---

    fn render_status_bar(&mut self, ctx: &egui::Context) {
        let mut switch_next_clicked = false;

        egui::TopBottomPanel::top("status_bar")
            .exact_height(30.0)
            .frame(
                egui::Frame::NONE
                    .fill(STATUS_BG)
                    .inner_margin(egui::Margin::symmetric(10, 4))
                    .stroke(Stroke::new(1.0_f32, BORDER_BLUE)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| match self.mode {
                    AppMode::Editor => {
                        // LZ badge
                        egui::Frame::NONE
                            .fill(STATUS_BADGE_BG)
                            .corner_radius(CornerRadius::same(2))
                            .inner_margin(egui::Margin::symmetric(6, 1))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("LZ")
                                        .font(FontId::monospace(13.0))
                                        .strong()
                                        .color(STATUS_BADGE_FG),
                                );
                            });

                        ui.add_space(6.0);

                        // Multi-buffer count indicator `[1/3]` (clickable to cycle)
                        let buf_badge =
                            format!("[{}/{}]", self.active_buf + 1, self.buffers.len());
                        let badge_resp = ui.add(
                            egui::Button::new(
                                egui::RichText::new(buf_badge)
                                    .font(FontId::monospace(13.0))
                                    .strong()
                                    .color(if self.buffers.len() > 1 {
                                        TEXT_AMBER
                                    } else {
                                        TEXT_CYAN
                                    }),
                            )
                            .fill(BG_TAB_INACTIVE)
                            .stroke(Stroke::new(1.0_f32, BORDER_BLUE))
                            .corner_radius(CornerRadius::same(2)),
                        );
                        if badge_resp.clicked() {
                            switch_next_clicked = true;
                        }
                        badge_resp.on_hover_text("Ctrl+W またはクリックでバッファ切替");

                        ui.add_space(6.0);

                        // File name & dirty flag '*'
                        let name = self.cur_buf().display_name();
                        ui.label(
                            egui::RichText::new(name)
                                .font(FontId::monospace(14.0))
                                .strong()
                                .color(TEXT_WHITE),
                        );
                        if self.cur_buf().dirty {
                            ui.label(
                                egui::RichText::new("*")
                                    .font(FontId::monospace(15.0))
                                    .strong()
                                    .color(TEXT_AMBER),
                            );
                        }

                        // Prefix key badge (^Q / ^K)
                        if self.prefix != PrefixKey::None {
                            ui.add_space(8.0);
                            let ptext = match self.prefix {
                                PrefixKey::CtrlQ => "^Q",
                                PrefixKey::CtrlK => "^K",
                                PrefixKey::None => "",
                            };
                            egui::Frame::NONE
                                .fill(TEXT_AMBER)
                                .corner_radius(CornerRadius::same(2))
                                .inner_margin(egui::Margin::symmetric(5, 1))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(ptext)
                                            .font(FontId::monospace(13.0))
                                            .strong()
                                            .color(STATUS_BADGE_FG),
                                    );
                                });
                        }

                        // Status message
                        if !self.status_message.is_empty() {
                            ui.add_space(10.0);
                            let msg_color = if self.status_message.contains("エラー") {
                                TEXT_AMBER
                            } else {
                                TEXT_CYAN
                            };
                            ui.label(
                                egui::RichText::new(&self.status_message)
                                    .font(FontId::monospace(13.0))
                                    .color(msg_color),
                            );
                        }

                        // Right-aligned encoding + line/column info
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let buf = self.cur_buf();
                            let pos_text = format!(
                                "Ln {:>4}/{:<4}  Col {:>3}",
                                buf.display_line_num(),
                                buf.lines.len(),
                                buf.display_col_num()
                            );
                            ui.label(
                                egui::RichText::new(pos_text)
                                    .font(FontId::monospace(13.5))
                                    .color(TEXT_CYAN),
                            );

                            ui.add_space(10.0);
                            ui.label(
                                egui::RichText::new("|")
                                    .font(FontId::monospace(13.0))
                                    .color(BORDER_BLUE),
                            );
                            ui.add_space(10.0);

                            let eol = if buf.line_ending == "\r\n" {
                                "CRLF"
                            } else {
                                "LF"
                            };
                            ui.label(
                                egui::RichText::new(format!("{} {}", buf.encoding, eol))
                                    .font(FontId::monospace(13.5))
                                    .color(TEXT_WHITE),
                            );

                            if self.ime_active || !self.ime_preedit.is_empty() {
                                ui.add_space(10.0);
                                egui::Frame::NONE
                                    .fill(PREEDIT_BG)
                                    .corner_radius(CornerRadius::same(2))
                                    .inner_margin(egui::Margin::symmetric(5, 1))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new("IME")
                                                .font(FontId::monospace(12.0))
                                                .color(PREEDIT_FG),
                                        );
                                    });
                            }
                        });
                    }
                    AppMode::Filer => {
                        // FILER badge
                        egui::Frame::NONE
                            .fill(STATUS_BADGE_FILER_BG)
                            .corner_radius(CornerRadius::same(2))
                            .inner_margin(egui::Margin::symmetric(6, 1))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("LZ FILER")
                                        .font(FontId::monospace(13.0))
                                        .strong()
                                        .color(STATUS_BADGE_FG),
                                );
                            });

                        ui.add_space(8.0);

                        // Open buffers summary
                        let open_summary = format!(
                            "編集中:{}件 ({})",
                            self.buffers.len(),
                            self.cur_buf().display_name()
                        );
                        ui.label(
                            egui::RichText::new(open_summary)
                                .font(FontId::monospace(13.0))
                                .color(TEXT_WHITE),
                        );

                        // Marked items indicator on active pane
                        let marked_count = self.filer.active_pane().marked.len();
                        if marked_count > 0 {
                            ui.add_space(10.0);
                            egui::Frame::NONE
                                .fill(FILER_ROW_MARKED_CURSOR)
                                .corner_radius(CornerRadius::same(2))
                                .inner_margin(egui::Margin::symmetric(6, 1))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("マーク: {marked_count}件"))
                                            .font(FontId::monospace(12.5))
                                            .strong()
                                            .color(TEXT_AMBER),
                                    );
                                });
                        }

                        if !self.status_message.is_empty() {
                            ui.add_space(10.0);
                            let msg_color = if self.status_message.contains("エラー")
                                || self.status_message.contains("失敗")
                            {
                                TEXT_AMBER
                            } else {
                                TEXT_CYAN
                            };
                            ui.label(
                                egui::RichText::new(&self.status_message)
                                    .font(FontId::monospace(13.0))
                                    .color(msg_color),
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let sort_label = self.filer.active_pane().sort_mode.label();
                            let side = match self.filer.active {
                                ActivePane::Left => "左窓",
                                ActivePane::Right => "右窓",
                            };
                            ui.label(
                                egui::RichText::new(format!("{side} [{sort_label}]"))
                                    .font(FontId::monospace(13.0))
                                    .color(TEXT_CYAN),
                            );
                        });
                    }
                });
            });

        if switch_next_clicked {
            self.switch_buffer_step(1);
        }
    }

    /// Compact multi-buffer strip displayed under the status bar in Editor mode when 2+ files are open.
    fn render_buffer_strip(&mut self, ctx: &egui::Context) {
        if self.mode != AppMode::Editor || self.buffers.len() < 2 {
            return;
        }
        let mut selected_idx: Option<usize> = None;

        egui::TopBottomPanel::top("buffer_strip")
            .exact_height(24.0)
            .frame(
                egui::Frame::NONE
                    .fill(BG_TAB_BAR)
                    .inner_margin(egui::Margin::symmetric(8, 2))
                    .stroke(Stroke::new(1.0_f32, GUTTER_SEP)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new("^W バッファ:")
                            .font(FontId::monospace(12.0))
                            .color(TEXT_DIM_CYAN),
                    );
                    ui.add_space(4.0);

                    for (idx, slot) in self.buffers.iter().enumerate() {
                        let is_active = idx == self.active_buf;
                        let dirty_mark = if slot.buffer.dirty { "*" } else { "" };
                        let title =
                            format!("{}:{}{}", idx + 1, slot.buffer.display_name(), dirty_mark);
                        let resp = ui.add(
                            egui::Button::new(
                                egui::RichText::new(title)
                                    .font(FontId::monospace(12.5))
                                    .color(if is_active {
                                        TEXT_WHITE
                                    } else {
                                        TEXT_DIM_CYAN
                                    }),
                            )
                            .fill(if is_active {
                                BG_TAB_ACTIVE
                            } else {
                                BG_TAB_INACTIVE
                            })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if is_active { TEXT_CYAN } else { GUTTER_SEP },
                            ))
                            .corner_radius(CornerRadius::same(2)),
                        );
                        if resp.clicked() {
                            selected_idx = Some(idx);
                        }
                    }
                });
            });

        if let Some(idx) = selected_idx {
            self.switch_buffer_to(idx);
        }
    }

    // --- Function Key Guide Bar (F1..F10, 2 blocks of 5 keys) ---

    fn render_fkey_bar(&mut self, ctx: &egui::Context) {
        match self.mode {
            AppMode::Editor => self.render_editor_fkey_bar(ctx),
            AppMode::Filer => self.render_filer_fkey_bar(ctx),
        }
    }

    fn render_editor_fkey_bar(&mut self, ctx: &egui::Context) {
        let keys_left: &[(&str, &str, EditorAction)] = &[
            ("F1", "ヘルプ", EditorAction::ToggleHelp),
            ("F2", "新規", EditorAction::NewBuffer),
            ("F3", "開く", EditorAction::OpenFile),
            ("F4", "行頭", EditorAction::MoveLineStart),
            ("F5", "行末", EditorAction::MoveLineEnd),
        ];
        let keys_right: &[(&str, &str, EditorAction)] = &[
            ("F6", "行削除", EditorAction::DeleteLine),
            ("F7", "単語左", EditorAction::MoveWordLeft),
            ("F8", "単語右", EditorAction::MoveWordRight),
            ("F9", "ファイラ", EditorAction::SwitchToFiler),
            ("F10", "保存", EditorAction::SaveFile),
        ];

        let mut clicked_action: Option<EditorAction> = None;

        egui::TopBottomPanel::bottom("fkey_guide_bar")
            .exact_height(34.0)
            .frame(
                egui::Frame::NONE
                    .fill(FKEY_BAR_BG)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .stroke(Stroke::new(1.0_f32, BORDER_BLUE)),
            )
            .show(ctx, |ui| {
                let avail_w = ui.available_width();
                let block_gap = 18.0_f32;
                let key_gap = 4.0_f32;
                let btn_w = ((avail_w - block_gap - key_gap * 8.0) / 10.0).max(56.0);
                let btn_h = ui.available_height().max(24.0);

                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = key_gap;

                    for &(fkey, label, action) in keys_left {
                        if Self::paint_fkey_button(
                            ui,
                            fkey,
                            label,
                            Vec2::new(btn_w, btn_h),
                            FKEY_NUM_BG,
                        ) {
                            clicked_action = Some(action);
                        }
                    }

                    ui.add_space(block_gap - key_gap);

                    for &(fkey, label, action) in keys_right {
                        if Self::paint_fkey_button(
                            ui,
                            fkey,
                            label,
                            Vec2::new(btn_w, btn_h),
                            FKEY_NUM_BG,
                        ) {
                            clicked_action = Some(action);
                        }
                    }
                });
            });

        if let Some(action) = clicked_action {
            self.execute_editor_action(action);
        }
    }

    fn render_filer_fkey_bar(&mut self, ctx: &egui::Context) {
        let keys_left: &[(&str, &str, FilerAction)] = &[
            ("F1", "ヘルプ", FilerAction::ToggleHelp),
            ("F2", "名前変", FilerAction::Rename),
            ("F3", "新規作成", FilerAction::NewFile),
            ("F4", "コピー", FilerAction::Copy),
            ("F5", "移動", FilerAction::Move),
        ];
        let keys_right: &[(&str, &str, FilerAction)] = &[
            ("F6", "削除", FilerAction::Delete),
            ("F7", "フォルダ", FilerAction::NewDir),
            ("F8", "ソート", FilerAction::CycleSort),
            ("F9", "編集戻", FilerAction::BackToEditor),
            ("F10", "終了", FilerAction::Quit),
        ];

        let mut clicked_action: Option<FilerAction> = None;

        egui::TopBottomPanel::bottom("fkey_guide_bar")
            .exact_height(34.0)
            .frame(
                egui::Frame::NONE
                    .fill(FKEY_BAR_BG)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .stroke(Stroke::new(1.0_f32, BORDER_BLUE)),
            )
            .show(ctx, |ui| {
                let avail_w = ui.available_width();
                let block_gap = 18.0_f32;
                let key_gap = 4.0_f32;
                let btn_w = ((avail_w - block_gap - key_gap * 8.0) / 10.0).max(56.0);
                let btn_h = ui.available_height().max(24.0);

                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = key_gap;

                    for &(fkey, label, action) in keys_left {
                        if Self::paint_fkey_button(
                            ui,
                            fkey,
                            label,
                            Vec2::new(btn_w, btn_h),
                            FKEY_NUM_FILER_BG,
                        ) {
                            clicked_action = Some(action);
                        }
                    }

                    ui.add_space(block_gap - key_gap);

                    for &(fkey, label, action) in keys_right {
                        if Self::paint_fkey_button(
                            ui,
                            fkey,
                            label,
                            Vec2::new(btn_w, btn_h),
                            FKEY_NUM_FILER_BG,
                        ) {
                            clicked_action = Some(action);
                        }
                    }
                });
            });

        if let Some(action) = clicked_action {
            self.execute_filer_action(ctx, action);
        }
    }

    fn paint_fkey_button(
        ui: &mut egui::Ui,
        fkey: &str,
        label: &str,
        size: Vec2,
        badge_color: Color32,
    ) -> bool {
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let bg = if response.hovered() {
                FKEY_BTN_HOVER
            } else {
                FKEY_BTN_BG
            };
            painter.rect(
                rect,
                CornerRadius::same(2),
                bg,
                Stroke::new(1.0_f32, BORDER_BLUE),
                StrokeKind::Inside,
            );

            let badge_w = (rect.width() * 0.34).clamp(24.0, 34.0);
            let badge_rect =
                Rect::from_min_max(rect.min, Pos2::new(rect.min.x + badge_w, rect.max.y));
            painter.rect_filled(badge_rect, CornerRadius::same(1), badge_color);
            painter.text(
                badge_rect.center(),
                Align2::CENTER_CENTER,
                fkey,
                FontId::monospace(11.5),
                FKEY_NUM_FG,
            );

            let label_center = Pos2::new(
                badge_rect.max.x + (rect.max.x - badge_rect.max.x) * 0.5,
                rect.center().y,
            );
            painter.text(
                label_center,
                Align2::CENTER_CENTER,
                label,
                FontId::monospace(12.0),
                TEXT_WHITE,
            );
        }
        response.clicked()
    }

    // --- Main Center View (Editor or 2-Pane Filer) ---

    fn render_editor_canvas(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG_NAVY))
            .show(ctx, |ui| {
                let avail_rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(avail_rect, Sense::click_and_drag());

                let scrollbar_w = 12.0;
                let total_lines = self.cur_buf().lines.len();
                let line_digits = total_lines.max(999).to_string().len();
                let gutter_w = (line_digits as f32 * 9.5 + 18.0).max(54.0);
                let top_pad = 6.0;
                let left_pad = 10.0;

                let editor_rect = Rect::from_min_max(
                    Pos2::new(avail_rect.min.x + gutter_w, avail_rect.min.y),
                    Pos2::new(avail_rect.max.x - scrollbar_w, avail_rect.max.y),
                );
                let viewport_h = (editor_rect.height() - top_pad * 2.0).max(self.line_height);
                let viewport_w = (editor_rect.width() - left_pad * 2.0).max(40.0);
                self.last_visible_lines = (viewport_h / self.line_height).floor().max(1.0) as usize;

                let font_id = FontId::monospace(self.font_size);

                // Measure cursor X position in pixels
                let cursor_line = self.cur_buf().cursor_line;
                let cursor_col = self.cur_buf().cursor_col;
                let cursor_line_str = self.cur_buf().current_line_str().to_string();
                let cursor_byte = char_to_byte_idx(&cursor_line_str, cursor_col);
                let before_cursor_disp = expand_tabs_for_display(&cursor_line_str[..cursor_byte]);
                let preedit_disp = expand_tabs_for_display(&self.ime_preedit);

                let (cursor_px_x, preedit_px_w) = ui.fonts(|f| {
                    let g_before =
                        f.layout_no_wrap(before_cursor_disp.clone(), font_id.clone(), TEXT_WHITE);
                    let x = g_before.size().x;
                    let pw = if preedit_disp.is_empty() {
                        0.0
                    } else {
                        f.layout_no_wrap(preedit_disp.clone(), font_id.clone(), PREEDIT_FG)
                            .size()
                            .x
                    };
                    (x, pw)
                });

                let mut scroll_y = self.cur_slot().scroll_y;
                let mut scroll_x = self.cur_slot().scroll_x;

                // Mouse wheel scrolling
                if response.hovered() {
                    let scroll_delta = ctx.input(|i| i.smooth_scroll_delta);
                    if scroll_delta.y.abs() > 0.0 {
                        scroll_y -= scroll_delta.y;
                    }
                    if scroll_delta.x.abs() > 0.0 {
                        scroll_x -= scroll_delta.x;
                    }
                }

                // Auto-scroll to keep cursor inside viewport
                let cursor_px_y = cursor_line as f32 * self.line_height;
                if self.need_scroll_to_cursor {
                    if cursor_px_y < scroll_y {
                        scroll_y = cursor_px_y;
                    } else if cursor_px_y + self.line_height > scroll_y + viewport_h {
                        scroll_y = cursor_px_y + self.line_height - viewport_h;
                    }

                    let caret_end_x = cursor_px_x + preedit_px_w + 16.0;
                    if cursor_px_x < scroll_x {
                        scroll_x = (cursor_px_x - 24.0).max(0.0);
                    } else if caret_end_x > scroll_x + viewport_w {
                        scroll_x = (caret_end_x - viewport_w).max(0.0);
                    }
                    self.need_scroll_to_cursor = false;
                }

                let total_content_h = (total_lines as f32 + 2.0) * self.line_height;
                let max_scroll_y = (total_content_h - viewport_h).max(0.0);
                scroll_y = scroll_y.clamp(0.0, max_scroll_y);
                scroll_x = scroll_x.max(0.0);

                // Click-to-place cursor
                if response.clicked() {
                    if let Some(pointer_pos) = response.interact_pointer_pos() {
                        if editor_rect.contains(pointer_pos) {
                            let rel_y = pointer_pos.y - editor_rect.min.y - top_pad + scroll_y;
                            let target_line = ((rel_y / self.line_height).floor().max(0.0) as usize)
                                .min(total_lines.saturating_sub(1));

                            let click_x =
                                (pointer_pos.x - editor_rect.min.x - left_pad + scroll_x).max(0.0);
                            let line_str = self.cur_buf().lines[target_line].clone();
                            let target_col = ui.fonts(|f| {
                                let mut best_col = 0usize;
                                let mut best_dist = click_x;
                                let mut acc = String::new();
                                for (idx, c) in line_str.chars().enumerate() {
                                    if c == '\t' {
                                        acc.push_str("    ");
                                    } else {
                                        acc.push(c);
                                    }
                                    let w = f
                                        .layout_no_wrap(acc.clone(), font_id.clone(), TEXT_WHITE)
                                        .size()
                                        .x;
                                    let dist = (w - click_x).abs();
                                    if dist < best_dist {
                                        best_dist = dist;
                                        best_col = idx + 1;
                                    }
                                    if w > click_x + self.font_size * 2.0 {
                                        break;
                                    }
                                }
                                best_col
                            });
                            self.prefix = PrefixKey::None;
                            self.cur_buf_mut().set_cursor(target_line, target_col);
                        }
                    }
                }

                // Paint background & gutter
                let painter = ui.painter_at(avail_rect);
                painter.rect_filled(avail_rect, CornerRadius::ZERO, BG_NAVY);

                let gutter_rect = Rect::from_min_max(
                    avail_rect.min,
                    Pos2::new(avail_rect.min.x + gutter_w, avail_rect.max.y),
                );
                painter.rect_filled(gutter_rect, CornerRadius::ZERO, BG_GUTTER);
                painter.line_segment(
                    [
                        Pos2::new(gutter_rect.max.x, avail_rect.min.y),
                        Pos2::new(gutter_rect.max.x, avail_rect.max.y),
                    ],
                    Stroke::new(1.0_f32, GUTTER_SEP),
                );

                // Virtualized line painting
                let first_line = (scroll_y / self.line_height).floor().max(0.0) as usize;
                let last_line = ((scroll_y + viewport_h) / self.line_height).ceil() as usize;
                let end_line = (last_line + 1).min(total_lines);

                let editor_painter = ui.painter_at(editor_rect);
                let gutter_painter = ui.painter_at(gutter_rect);
                let cur_line_now = self.cur_buf().cursor_line;
                let cur_col_now = self.cur_buf().cursor_col;

                for line_idx in first_line..end_line {
                    let y = editor_rect.min.y + top_pad + line_idx as f32 * self.line_height
                        - scroll_y;
                    let is_current = line_idx == cur_line_now;

                    if is_current {
                        let line_bg_rect = Rect::from_min_size(
                            Pos2::new(editor_rect.min.x, y),
                            Vec2::new(editor_rect.width(), self.line_height),
                        );
                        editor_painter.rect_filled(
                            line_bg_rect,
                            CornerRadius::ZERO,
                            BG_ACTIVE_LINE,
                        );
                    }

                    let num_str = format!("{:>width$}", line_idx + 1, width = line_digits);
                    gutter_painter.text(
                        Pos2::new(gutter_rect.max.x - 8.0, y + self.line_height * 0.5),
                        Align2::RIGHT_CENTER,
                        num_str,
                        FontId::monospace(self.font_size - 2.0),
                        if is_current { TEXT_CYAN } else { TEXT_DIM_CYAN },
                    );

                    let text_origin_x = editor_rect.min.x + left_pad - scroll_x;
                    let text_y = y + (self.line_height - self.font_size) * 0.42;
                    let line = &self.cur_buf().lines[line_idx];

                    if is_current && !self.ime_preedit.is_empty() {
                        let byte_pos = char_to_byte_idx(line, cur_col_now);
                        let before = expand_tabs_for_display(&line[..byte_pos]);
                        let after = expand_tabs_for_display(&line[byte_pos..]);
                        let mut cur_x = text_origin_x;

                        if !before.is_empty() {
                            let g = editor_painter.layout_no_wrap(
                                before,
                                font_id.clone(),
                                TEXT_WHITE,
                            );
                            let w = g.size().x;
                            editor_painter.galley(Pos2::new(cur_x, text_y), g, TEXT_WHITE);
                            cur_x += w;
                        }

                        let g_pre = editor_painter.layout_no_wrap(
                            preedit_disp.clone(),
                            font_id.clone(),
                            PREEDIT_FG,
                        );
                        let pw = g_pre.size().x;
                        let pre_rect = Rect::from_min_size(
                            Pos2::new(cur_x, y + 1.0),
                            Vec2::new(pw.max(4.0), self.line_height - 2.0),
                        );
                        editor_painter.rect_filled(pre_rect, CornerRadius::same(2), PREEDIT_BG);
                        editor_painter.galley(Pos2::new(cur_x, text_y), g_pre, PREEDIT_FG);
                        editor_painter.line_segment(
                            [
                                Pos2::new(cur_x, y + self.line_height - 2.0),
                                Pos2::new(cur_x + pw, y + self.line_height - 2.0),
                            ],
                            Stroke::new(1.5_f32, PREEDIT_UNDERLINE),
                        );
                        cur_x += pw;

                        if !after.is_empty() {
                            let g_after = editor_painter.layout_no_wrap(
                                after,
                                font_id.clone(),
                                TEXT_WHITE,
                            );
                            let w = g_after.size().x;
                            editor_painter.galley(Pos2::new(cur_x, text_y), g_after, TEXT_WHITE);
                            cur_x += w;
                        }

                        if line_idx + 1 < total_lines {
                            editor_painter.text(
                                Pos2::new(cur_x + 2.0, y + self.line_height * 0.5),
                                Align2::LEFT_CENTER,
                                "↵",
                                FontId::monospace(self.font_size - 2.0),
                                TEXT_CR_MARK,
                            );
                        }
                    } else {
                        let disp = expand_tabs_for_display(line);
                        let galley =
                            editor_painter.layout_no_wrap(disp, font_id.clone(), TEXT_WHITE);
                        let end_x = text_origin_x + galley.size().x;
                        editor_painter.galley(Pos2::new(text_origin_x, text_y), galley, TEXT_WHITE);

                        if line_idx + 1 < total_lines {
                            editor_painter.text(
                                Pos2::new(end_x + 2.0, y + self.line_height * 0.5),
                                Align2::LEFT_CENTER,
                                "↵",
                                FontId::monospace(self.font_size - 2.0),
                                TEXT_CR_MARK,
                            );
                        }
                    }
                }

                // [EOF] marker
                let eof_y = editor_rect.min.y
                    + top_pad
                    + total_lines as f32 * self.line_height
                    - scroll_y;
                if eof_y >= editor_rect.min.y - self.line_height && eof_y <= editor_rect.max.y {
                    editor_painter.text(
                        Pos2::new(
                            editor_rect.min.x + left_pad - scroll_x,
                            eof_y + self.line_height * 0.5,
                        ),
                        Align2::LEFT_CENTER,
                        "[EOF]",
                        FontId::monospace(self.font_size - 2.0),
                        TEXT_EOF,
                    );
                }

                // Cursor rendering
                let caret_x = editor_rect.min.x + left_pad + cursor_px_x + preedit_px_w - scroll_x;
                let caret_y = editor_rect.min.y + top_pad + cursor_px_y - scroll_y;
                let cursor_rect = Rect::from_min_size(
                    Pos2::new(caret_x, caret_y + 1.0),
                    Vec2::new(9.0, self.line_height - 2.0),
                );

                if editor_rect.intersects(cursor_rect) && self.inline_prompt.is_none() {
                    let t = ctx.input(|i| i.time);
                    let blink_on = !self.ime_preedit.is_empty() || ((t * 1.8) as u64 % 2 == 0);
                    if blink_on {
                        editor_painter.rect_filled(cursor_rect, CornerRadius::ZERO, CURSOR_FILL);
                        editor_painter.line_segment(
                            [
                                Pos2::new(caret_x, caret_y + 1.0),
                                Pos2::new(caret_x, caret_y + self.line_height - 1.0),
                            ],
                            Stroke::new(2.0_f32, CURSOR_BAR),
                        );
                    }
                    ctx.request_repaint_after(std::time::Duration::from_millis(450));
                }

                // Report exact IME cursor area for Fcitx5
                if self.inline_prompt.is_none() {
                    let preedit_start_x =
                        editor_rect.min.x + left_pad + cursor_px_x - scroll_x;
                    let ime_x = preedit_start_x.clamp(editor_rect.min.x, editor_rect.max.x - 4.0);
                    let ime_y =
                        caret_y.clamp(editor_rect.min.y, editor_rect.max.y - self.line_height);
                    let ime_rect = Rect::from_min_size(
                        Pos2::new(ime_x, ime_y),
                        Vec2::new(preedit_px_w.max(2.0), self.line_height),
                    );
                    ctx.output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            rect: ime_rect,
                            cursor_rect: ime_rect,
                        });
                    });
                }

                // Vertical scrollbar
                let sb_rect = Rect::from_min_max(
                    Pos2::new(avail_rect.max.x - scrollbar_w, avail_rect.min.y),
                    avail_rect.max,
                );
                painter.rect_filled(sb_rect, CornerRadius::ZERO, BG_GUTTER);
                if max_scroll_y > 0.0 {
                    let ratio = (viewport_h / total_content_h).clamp(0.06, 1.0);
                    let thumb_h = (sb_rect.height() * ratio).max(24.0);
                    let track_h = (sb_rect.height() - thumb_h).max(1.0);
                    let thumb_y = sb_rect.min.y + (scroll_y / max_scroll_y) * track_h;
                    let thumb_rect = Rect::from_min_size(
                        Pos2::new(sb_rect.min.x + 2.0, thumb_y),
                        Vec2::new(scrollbar_w - 4.0, thumb_h),
                    );
                    painter.rect_filled(thumb_rect, CornerRadius::same(3), BORDER_BLUE);

                    let sb_resp =
                        ui.interact(sb_rect, ui.id().with("vscroll"), Sense::click_and_drag());
                    if sb_resp.dragged() || sb_resp.clicked() {
                        if let Some(pos) = sb_resp.interact_pointer_pos() {
                            let norm =
                                ((pos.y - sb_rect.min.y - thumb_h * 0.5) / track_h).clamp(0.0, 1.0);
                            scroll_y = norm * max_scroll_y;
                        }
                    }
                }

                let slot = self.cur_slot_mut();
                slot.scroll_x = scroll_x;
                slot.scroll_y = scroll_y;
            });
    }

    fn render_filer_canvas(&mut self, ctx: &egui::Context) {
        // When browsing in Filer (no modal open), disable IME so Space/Tab/ESC/VZ keys work cleanly.
        if self.filer_modal.is_none() {
            ctx.output_mut(|o| {
                o.ime = None;
            });
        }

        let mut open_double_clicked = false;

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(BG_NAVY)
                    .inner_margin(egui::Margin::same(6)),
            )
            .show(ctx, |ui| {
                let avail = ui.available_rect_before_wrap();
                let gap = 6.0_f32;
                let half_w = ((avail.width() - gap) * 0.5).max(200.0);

                let left_rect =
                    Rect::from_min_size(avail.min, Vec2::new(half_w, avail.height()));
                let right_rect = Rect::from_min_size(
                    Pos2::new(avail.min.x + half_w + gap, avail.min.y),
                    Vec2::new(avail.width() - half_w - gap, avail.height()),
                );

                if self.paint_filer_pane(ui, ctx, ActivePane::Left, left_rect) {
                    open_double_clicked = true;
                }
                if self.paint_filer_pane(ui, ctx, ActivePane::Right, right_rect) {
                    open_double_clicked = true;
                }
            });

        if open_double_clicked {
            self.filer_open_selected();
        }
    }

    /// Paint one side (Left or Right) of the 2-pane VZ Filer. Returns true if an item was double-clicked.
    fn paint_filer_pane(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        side: ActivePane,
        rect: Rect,
    ) -> bool {
        let is_active = self.filer.active == side;
        let pane_id = ui.id().with(match side {
            ActivePane::Left => "filer_pane_left",
            ActivePane::Right => "filer_pane_right",
        });

        let header_h = 26.0_f32;
        let col_hdr_h = 22.0_f32;
        let row_h = 22.0_f32;
        let sb_w = 10.0_f32;

        let border_color = if is_active {
            BORDER_ACTIVE_PANE
        } else {
            BORDER_BLUE
        };
        let hdr_bg = if is_active {
            FILER_HDR_ACTIVE
        } else {
            FILER_HDR_INACTIVE
        };

        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            CornerRadius::same(2),
            BG_NAVY,
            Stroke::new(if is_active { 1.5_f32 } else { 1.0_f32 }, border_color),
            StrokeKind::Inside,
        );

        // Directory Header Bar
        let hdr_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + 1.0, rect.min.y + 1.0),
            Vec2::new(rect.width() - 2.0, header_h),
        );
        painter.rect_filled(hdr_rect, CornerRadius::ZERO, hdr_bg);

        let pane = match side {
            ActivePane::Left => &mut self.filer.left,
            ActivePane::Right => &mut self.filer.right,
        };

        let side_tag = match side {
            ActivePane::Left => "[L]",
            ActivePane::Right => "[R]",
        };
        let max_dir_cols = ((hdr_rect.width() - 96.0) / 8.0).floor().max(12.0) as usize;
        let dir_str = truncate_path_left(&pane.current_dir.display().to_string(), max_dir_cols);
        let entry_count = pane.entries.iter().filter(|e| !e.is_parent).count();
        let marked_count = pane.marked.len();

        painter.text(
            Pos2::new(hdr_rect.min.x + 8.0, hdr_rect.center().y),
            Align2::LEFT_CENTER,
            format!("{side_tag} {dir_str}"),
            FontId::monospace(13.0),
            if is_active { TEXT_CYAN } else { TEXT_WHITE },
        );

        let right_info = if marked_count > 0 {
            format!("*{marked_count}/{entry_count}")
        } else {
            format!("{entry_count}件")
        };
        painter.text(
            Pos2::new(hdr_rect.max.x - 8.0, hdr_rect.center().y),
            Align2::RIGHT_CENTER,
            right_info,
            FontId::monospace(12.5),
            if marked_count > 0 {
                TEXT_AMBER
            } else {
                TEXT_DIM_CYAN
            },
        );

        // Column Header Row
        let col_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + 1.0, hdr_rect.max.y),
            Vec2::new(rect.width() - 2.0, col_hdr_h),
        );
        painter.rect_filled(col_rect, CornerRadius::ZERO, FILER_COL_HDR);
        painter.line_segment(
            [
                Pos2::new(col_rect.min.x, col_rect.max.y),
                Pos2::new(col_rect.max.x, col_rect.max.y),
            ],
            Stroke::new(1.0_f32, GUTTER_SEP),
        );

        let list_rect = Rect::from_min_max(
            Pos2::new(rect.min.x + 1.0, col_rect.max.y + 1.0),
            Pos2::new(rect.max.x - sb_w - 1.0, rect.max.y - 1.0),
        );

        let date_col_w = 136.0_f32;
        let size_col_w = 82.0_f32;
        let date_x = list_rect.max.x - 8.0;
        let size_x = date_x - date_col_w - 8.0;
        let name_max_w = (size_x - size_col_w - list_rect.min.x - 24.0).max(80.0);

        painter.text(
            Pos2::new(list_rect.min.x + 22.0, col_rect.center().y),
            Align2::LEFT_CENTER,
            "名前",
            FontId::monospace(12.0),
            TEXT_DIM_CYAN,
        );
        painter.text(
            Pos2::new(size_x, col_rect.center().y),
            Align2::RIGHT_CENTER,
            "サイズ",
            FontId::monospace(12.0),
            TEXT_DIM_CYAN,
        );
        painter.text(
            Pos2::new(date_x, col_rect.center().y),
            Align2::RIGHT_CENTER,
            "更新日時",
            FontId::monospace(12.0),
            TEXT_DIM_CYAN,
        );

        // Interaction area for the pane
        let resp = ui.interact(list_rect, pane_id, Sense::click());
        let visible_rows = (list_rect.height() / row_h).floor().max(1.0) as usize;
        if is_active {
            self.last_filer_visible_rows = visible_rows;
        }

        // Scroll handling
        let pane = match side {
            ActivePane::Left => &mut self.filer.left,
            ActivePane::Right => &mut self.filer.right,
        };
        if resp.hovered() {
            let delta = ctx.input(|i| i.smooth_scroll_delta);
            if delta.y.abs() > 0.0 {
                pane.scroll_y -= delta.y;
            }
        }

        if pane.need_scroll_to_cursor {
            let cy = pane.cursor as f32 * row_h;
            if cy < pane.scroll_y {
                pane.scroll_y = cy;
            } else if cy + row_h > pane.scroll_y + list_rect.height() {
                pane.scroll_y = cy + row_h - list_rect.height();
            }
            pane.need_scroll_to_cursor = false;
        }

        let total_h = pane.entries.len() as f32 * row_h;
        let max_scroll = (total_h - list_rect.height()).max(0.0);
        pane.scroll_y = pane.scroll_y.clamp(0.0, max_scroll);

        // Mouse click / double-click on rows
        let mut double_clicked = false;
        if resp.clicked() || resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let clicked_row =
                    ((pos.y - list_rect.min.y + pane.scroll_y) / row_h).floor().max(0.0) as usize;
                if clicked_row < pane.entries.len() {
                    pane.cursor = clicked_row;
                    if resp.double_clicked() {
                        double_clicked = true;
                    }
                }
            }
            self.filer.active = side;
        }

        // Re-borrow pane immutably for painting
        let pane = match side {
            ActivePane::Left => &self.filer.left,
            ActivePane::Right => &self.filer.right,
        };
        let is_active_now = self.filer.active == side;
        let list_painter = ui.painter_at(list_rect);

        let first_row = (pane.scroll_y / row_h).floor().max(0.0) as usize;
        let last_row = ((pane.scroll_y + list_rect.height()) / row_h).ceil() as usize;
        let end_row = (last_row + 1).min(pane.entries.len());

        for idx in first_row..end_row {
            let entry = &pane.entries[idx];
            let y = list_rect.min.y + idx as f32 * row_h - pane.scroll_y;
            let row_rect = Rect::from_min_size(
                Pos2::new(list_rect.min.x, y),
                Vec2::new(list_rect.width(), row_h),
            );

            let is_cursor = idx == pane.cursor;
            let is_marked = !entry.is_parent && pane.marked.contains(&entry.path);

            let row_bg = match (is_cursor, is_marked, is_active_now) {
                (true, true, true) => Some(FILER_ROW_MARKED_CURSOR),
                (true, false, true) => Some(FILER_ROW_CURSOR_ACTIVE),
                (true, _, false) => Some(FILER_ROW_CURSOR_INACTIVE),
                (false, true, _) => Some(FILER_ROW_MARKED),
                (false, false, _) => None,
            };
            if let Some(bg) = row_bg {
                list_painter.rect_filled(row_rect, CornerRadius::ZERO, bg);
            }
            if is_cursor && is_active_now {
                list_painter.rect_filled(
                    Rect::from_min_size(row_rect.min, Vec2::new(3.0, row_h)),
                    CornerRadius::ZERO,
                    CURSOR_BAR,
                );
            }

            // Mark asterisk
            if is_marked {
                list_painter.text(
                    Pos2::new(list_rect.min.x + 9.0, row_rect.center().y),
                    Align2::LEFT_CENTER,
                    "*",
                    FontId::monospace(14.0),
                    TEXT_AMBER,
                );
            }

            // Name display
            let display_name = if entry.is_parent {
                "..".to_string()
            } else if entry.is_dir {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            };
            let max_chars = (name_max_w / 8.5).floor().max(10.0) as usize;
            let clipped_name = truncate_display_right(&display_name, max_chars);

            let name_color = if is_marked {
                TEXT_AMBER
            } else if entry.is_dir {
                TEXT_CYAN
            } else {
                TEXT_WHITE
            };
            list_painter.text(
                Pos2::new(list_rect.min.x + 22.0, row_rect.center().y),
                Align2::LEFT_CENTER,
                clipped_name,
                FontId::monospace(14.0),
                name_color,
            );

            // Size column
            let size_text = if entry.is_parent {
                "<UP>".to_string()
            } else if entry.is_dir {
                "<DIR>".to_string()
            } else {
                format_size(entry.size)
            };
            list_painter.text(
                Pos2::new(size_x, row_rect.center().y),
                Align2::RIGHT_CENTER,
                size_text,
                FontId::monospace(12.5),
                if entry.is_dir {
                    TEXT_DIM_CYAN
                } else {
                    TEXT_WHITE
                },
            );

            // Modified timestamp
            if !entry.modified_str.is_empty() {
                list_painter.text(
                    Pos2::new(date_x, row_rect.center().y),
                    Align2::RIGHT_CENTER,
                    &entry.modified_str,
                    FontId::monospace(12.0),
                    TEXT_DIM_CYAN,
                );
            }
        }

        // Scrollbar for pane
        let sb_rect = Rect::from_min_max(
            Pos2::new(rect.max.x - sb_w - 1.0, col_rect.max.y + 1.0),
            Pos2::new(rect.max.x - 1.0, rect.max.y - 1.0),
        );
        painter.rect_filled(sb_rect, CornerRadius::ZERO, BG_GUTTER);
        if max_scroll > 0.0 {
            let ratio = (list_rect.height() / total_h).clamp(0.08, 1.0);
            let thumb_h = (sb_rect.height() * ratio).max(20.0);
            let track_h = (sb_rect.height() - thumb_h).max(1.0);
            let thumb_y = sb_rect.min.y + (pane.scroll_y / max_scroll) * track_h;
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(sb_rect.min.x + 2.0, thumb_y),
                    Vec2::new(sb_w - 4.0, thumb_h),
                ),
                CornerRadius::same(2),
                if is_active_now {
                    BORDER_ACTIVE_PANE
                } else {
                    BORDER_BLUE
                },
            );
        }

        double_clicked
    }

    // --- Overlays & Modals ---

    fn render_help_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let mut open = self.show_help;
        let title = match self.mode {
            AppMode::Editor => "LZ Editor - エディタ キーバインド (F1 / ESC で閉じる)",
            AppMode::Filer => "LZ Filer - 2画面ファイラー操作 (F1 / ESC で閉じる)",
        };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(STATUS_BG)
                    .stroke(Stroke::new(1.5_f32, TEXT_CYAN)),
            )
            .show(ctx, |ui| {
                let rows: &[(&str, &str)] = match self.mode {
                    AppMode::Editor => &[
                        ("Ctrl+E / S / D / X", "ダイヤモンドカーソル移動 (上/左/右/下)"),
                        ("Ctrl+A / Ctrl+F", "単語単位の左・右移動"),
                        ("Ctrl+Q+S / Ctrl+Q+D", "行頭・行末移動 (Home / End も可)"),
                        ("Ctrl+Q+R / Ctrl+Q+C", "ファイル先頭・末尾移動"),
                        ("Ctrl+Y", "現在行の削除"),
                        ("Ctrl+G / Delete", "カーソル位置の1文字削除"),
                        ("BackSpace / Ctrl+H", "前1文字削除"),
                        ("Ctrl+W / Ctrl+Shift+W", "複数バッファ（裏画面）切替"),
                        ("Ctrl+K+D / F10", "上書き保存 / 保存ダイアログ"),
                        ("ESC / Ctrl+K+Q / F9", "2画面ファイラーへ切替"),
                    ],
                    AppMode::Filer => &[
                        ("Ctrl+E / X (↑ / ↓)", "ファイルカーソル移動"),
                        ("Tab / Ctrl+S / D (← / →)", "左右ペイン（アクティブ窓）切替"),
                        ("Enter / ダブルクリック", "フォルダ移動・ファイルをバッファで開く"),
                        ("BackSpace / Ctrl+H", "親ディレクトリへ移動"),
                        ("Space", "ファイルマーク（複数選択・反転表示）"),
                        ("F2 / F3 / F7", "リネーム / 新規ファイル作成 / フォルダ作成"),
                        ("F4 / F5 / F6", "反対側窓へコピー / 移動 / 削除"),
                        ("F8", "ソート順切替（名前/更新/サイズ/拡張子）"),
                        ("ESC / F9", "エディタ画面へ戻る"),
                        ("F10", "アプリケーション終了"),
                    ],
                };
                egui::Grid::new("vz_help_grid")
                    .num_columns(2)
                    .spacing([18.0, 6.0])
                    .show(ui, |ui| {
                        for &(k, desc) in rows {
                            ui.label(
                                egui::RichText::new(k)
                                    .font(FontId::monospace(13.5))
                                    .strong()
                                    .color(TEXT_CYAN),
                            );
                            ui.label(
                                egui::RichText::new(desc)
                                    .font(FontId::monospace(13.5))
                                    .color(TEXT_WHITE),
                            );
                            ui.end_row();
                        }
                    });
            });
        self.show_help = open;
    }

    fn render_filer_modal(&mut self, ctx: &egui::Context) {
        let Some(mut modal) = self.filer_modal.take() else {
            return;
        };

        let mut keep_modal = true;

        match &mut modal {
            FilerModal::Rename {
                src,
                input,
                just_opened,
            } => {
                let mut do_submit = false;
                let mut do_cancel = false;
                egui::Window::new("リネーム (F2)")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(STATUS_BG)
                            .stroke(Stroke::new(1.5_f32, TEXT_CYAN)),
                    )
                    .show(ctx, |ui| {
                        ui.set_min_width(420.0);
                        ui.label(
                            egui::RichText::new(format!("対象: {}", src.display()))
                                .font(FontId::monospace(12.5))
                                .color(TEXT_DIM_CYAN),
                        );
                        ui.add_space(4.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(input)
                                .font(FontId::monospace(14.0))
                                .desired_width(400.0),
                        );
                        if *just_opened {
                            resp.request_focus();
                            *just_opened = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            do_submit = true;
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("変更 (Enter)").clicked() {
                                do_submit = true;
                            }
                            if ui.button("キャンセル (ESC)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape))
                            {
                                do_cancel = true;
                            }
                        });
                    });

                if do_submit {
                    keep_modal = false;
                    match rename_entry(src, input) {
                        Ok(new_path) => {
                            self.update_open_buffer_path(src, &new_path);
                            self.filer.refresh_both();
                            self.status_message =
                                format!("リネームしました: {}", new_path.display());
                        }
                        Err(e) => {
                            self.status_message = format!("リネームエラー: {e}");
                        }
                    }
                } else if do_cancel {
                    keep_modal = false;
                    self.status_message = "リネームをキャンセルしました".to_string();
                }
            }

            FilerModal::NewFile {
                dir,
                input,
                just_opened,
            } => {
                let mut do_submit = false;
                let mut do_cancel = false;
                egui::Window::new("新規ファイル作成 (F3)")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(STATUS_BG)
                            .stroke(Stroke::new(1.5_f32, TEXT_CYAN)),
                    )
                    .show(ctx, |ui| {
                        ui.set_min_width(420.0);
                        ui.label(
                            egui::RichText::new(format!("作成先: {}", dir.display()))
                                .font(FontId::monospace(12.5))
                                .color(TEXT_DIM_CYAN),
                        );
                        ui.add_space(4.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(input)
                                .hint_text("新しいファイル名を入力 (例: memo.txt)")
                                .font(FontId::monospace(14.0))
                                .desired_width(400.0),
                        );
                        if *just_opened {
                            resp.request_focus();
                            *just_opened = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            do_submit = true;
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("作成して開く (Enter)").clicked() {
                                do_submit = true;
                            }
                            if ui.button("キャンセル (ESC)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape))
                            {
                                do_cancel = true;
                            }
                        });
                    });

                if do_submit {
                    keep_modal = false;
                    match create_file(dir, input) {
                        Ok(created_path) => {
                            self.filer.refresh_both();
                            self.open_file_in_buffer(created_path);
                        }
                        Err(e) => {
                            self.status_message = format!("新規作成エラー: {e}");
                        }
                    }
                } else if do_cancel {
                    keep_modal = false;
                    self.status_message = "新規作成をキャンセルしました".to_string();
                }
            }

            FilerModal::NewDir {
                dir,
                input,
                just_opened,
            } => {
                let mut do_submit = false;
                let mut do_cancel = false;
                egui::Window::new("新規フォルダ作成 (F7)")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(STATUS_BG)
                            .stroke(Stroke::new(1.5_f32, TEXT_CYAN)),
                    )
                    .show(ctx, |ui| {
                        ui.set_min_width(420.0);
                        ui.label(
                            egui::RichText::new(format!("作成先: {}", dir.display()))
                                .font(FontId::monospace(12.5))
                                .color(TEXT_DIM_CYAN),
                        );
                        ui.add_space(4.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(input)
                                .hint_text("新しいフォルダ名を入力")
                                .font(FontId::monospace(14.0))
                                .desired_width(400.0),
                        );
                        if *just_opened {
                            resp.request_focus();
                            *just_opened = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            do_submit = true;
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("作成 (Enter)").clicked() {
                                do_submit = true;
                            }
                            if ui.button("キャンセル (ESC)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape))
                            {
                                do_cancel = true;
                            }
                        });
                    });

                if do_submit {
                    keep_modal = false;
                    match create_dir(dir, input) {
                        Ok(created_dir) => {
                            self.filer.refresh_both();
                            self.status_message =
                                format!("フォルダを作成しました: {}", created_dir.display());
                        }
                        Err(e) => {
                            self.status_message = format!("フォルダ作成エラー: {e}");
                        }
                    }
                } else if do_cancel {
                    keep_modal = false;
                    self.status_message = "フォルダ作成をキャンセルしました".to_string();
                }
            }

            FilerModal::ConfirmCopyOrMove {
                is_move,
                targets,
                dst_dir,
            } => {
                let op_name = if *is_move { "移動" } else { "コピー" };
                let mut do_exec = false;
                let mut do_cancel = false;

                egui::Window::new(format!("ファイル{op_name}確認"))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(STATUS_BG)
                            .stroke(Stroke::new(1.5_f32, TEXT_CYAN)),
                    )
                    .show(ctx, |ui| {
                        ui.set_min_width(440.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{}件の項目を反対側ペインへ{op_name}しますか？",
                                targets.len()
                            ))
                            .font(FontId::monospace(14.0))
                            .color(TEXT_WHITE),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("宛先: {}", dst_dir.display()))
                                .font(FontId::monospace(13.0))
                                .color(TEXT_CYAN),
                        );
                        ui.add_space(4.0);
                        for entry in targets.iter().take(6) {
                            ui.label(
                                egui::RichText::new(format!("  - {}", entry.name))
                                    .font(FontId::monospace(12.5))
                                    .color(TEXT_AMBER),
                            );
                        }
                        if targets.len() > 6 {
                            ui.label(
                                egui::RichText::new(format!("  ... 他 {}件", targets.len() - 6))
                                    .font(FontId::monospace(12.5))
                                    .color(TEXT_DIM_CYAN),
                            );
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button(format!("実行 (Enter / Y)")).clicked()
                                || ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Y))
                            {
                                do_exec = true;
                            }
                            if ui.button("キャンセル (ESC / N)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::N))
                            {
                                do_cancel = true;
                            }
                        });
                    });

                if do_exec {
                    keep_modal = false;
                    let mut ok_count = 0usize;
                    let mut last_err: Option<String> = None;
                    for entry in targets.iter() {
                        let res = if *is_move {
                            move_entry(&entry.path, dst_dir)
                        } else {
                            copy_entry(&entry.path, dst_dir)
                        };
                        match res {
                            Ok(new_path) => {
                                if *is_move {
                                    self.update_open_buffer_path(&entry.path, &new_path);
                                }
                                ok_count += 1;
                            }
                            Err(e) => {
                                last_err = Some(format!("{}: {e}", entry.name));
                            }
                        }
                    }
                    self.filer.active_pane_mut().marked.clear();
                    self.filer.refresh_both();
                    if let Some(err) = last_err {
                        self.status_message =
                            format!("{op_name}: {ok_count}件完了 / エラー({err})");
                    } else {
                        self.status_message = format!("{ok_count}件を{op_name}しました");
                    }
                } else if do_cancel {
                    keep_modal = false;
                    self.status_message = format!("{op_name}をキャンセルしました");
                }
            }

            FilerModal::ConfirmDelete { targets } => {
                let mut do_exec = false;
                let mut do_cancel = false;

                egui::Window::new("削除確認 (F6)")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(STATUS_BG)
                            .stroke(Stroke::new(1.5_f32, TEXT_AMBER)),
                    )
                    .show(ctx, |ui| {
                        ui.set_min_width(420.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "以下の {}件 を完全に削除します。よろしいですか？",
                                targets.len()
                            ))
                            .font(FontId::monospace(14.0))
                            .color(TEXT_AMBER),
                        );
                        ui.add_space(4.0);
                        for entry in targets.iter().take(6) {
                            let kind = if entry.is_dir { "[DIR] " } else { "" };
                            ui.label(
                                egui::RichText::new(format!("  - {kind}{}", entry.name))
                                    .font(FontId::monospace(13.0))
                                    .color(TEXT_WHITE),
                            );
                        }
                        if targets.len() > 6 {
                            ui.label(
                                egui::RichText::new(format!("  ... 他 {}件", targets.len() - 6))
                                    .font(FontId::monospace(12.5))
                                    .color(TEXT_DIM_CYAN),
                            );
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("削除実行 (Enter / Y)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Y))
                            {
                                do_exec = true;
                            }
                            if ui.button("キャンセル (ESC / N)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::N))
                            {
                                do_cancel = true;
                            }
                        });
                    });

                if do_exec {
                    keep_modal = false;
                    let mut ok_count = 0usize;
                    let mut last_err: Option<String> = None;
                    for entry in targets.iter() {
                        match delete_entry(&entry.path) {
                            Ok(()) => ok_count += 1,
                            Err(e) => last_err = Some(format!("{}: {e}", entry.name)),
                        }
                    }
                    self.filer.active_pane_mut().marked.clear();
                    self.filer.refresh_both();
                    if let Some(err) = last_err {
                        self.status_message = format!("削除: {ok_count}件完了 / エラー({err})");
                    } else {
                        self.status_message = format!("{ok_count}件を削除しました");
                    }
                } else if do_cancel {
                    keep_modal = false;
                    self.status_message = "削除をキャンセルしました".to_string();
                }
            }

            FilerModal::ConfirmQuit => {
                let mut do_quit = false;
                let mut do_cancel = false;
                egui::Window::new("終了確認 (F10)")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(STATUS_BG)
                            .stroke(Stroke::new(1.5_f32, TEXT_AMBER)),
                    )
                    .show(ctx, |ui| {
                        ui.set_min_width(380.0);
                        ui.label(
                            egui::RichText::new("未保存の変更があるバッファがあります。終了しますか？")
                                .font(FontId::monospace(13.5))
                                .color(TEXT_AMBER),
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("保存せず終了 (Enter / Y)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Y))
                            {
                                do_quit = true;
                            }
                            if ui.button("キャンセル (ESC / N)").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::N))
                            {
                                do_cancel = true;
                            }
                        });
                    });

                if do_quit {
                    keep_modal = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else if do_cancel {
                    keep_modal = false;
                    self.status_message = "終了をキャンセルしました".to_string();
                }
            }
        }

        if keep_modal {
            self.filer_modal = Some(modal);
        }
    }

    fn render_inline_path_prompt(&mut self, ctx: &egui::Context) {
        let mut submit_path: Option<(bool, PathBuf)> = None;
        let mut cancel = false;

        if let Some(prompt) = &mut self.inline_prompt {
            let title = if prompt.is_save {
                "名前を付けて保存 (ファイルパスを入力)"
            } else {
                "ファイルを開く (ファイルパスを入力)"
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(
                    egui::Frame::window(&ctx.style())
                        .fill(STATUS_BG)
                        .stroke(Stroke::new(1.5_f32, TEXT_CYAN)),
                )
                .show(ctx, |ui| {
                    ui.set_min_width(440.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut prompt.path_input)
                            .font(FontId::monospace(14.0))
                            .desired_width(420.0),
                    );
                    if prompt.just_opened {
                        resp.request_focus();
                        prompt.just_opened = false;
                    }

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("実行 (Enter)").clicked()
                            || (resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
                        {
                            let trimmed = prompt.path_input.trim();
                            if !trimmed.is_empty() {
                                submit_path = Some((prompt.is_save, PathBuf::from(trimmed)));
                            }
                        }
                        if ui.button("キャンセル (ESC)").clicked()
                            || ui.input(|i| i.key_pressed(Key::Escape))
                        {
                            cancel = true;
                        }
                    });
                });
        }

        if let Some((is_save, path)) = submit_path {
            self.inline_prompt = None;
            if is_save {
                self.perform_save(path);
            } else {
                self.open_file_in_buffer(path);
            }
        } else if cancel {
            self.inline_prompt = None;
            self.status_message = "キャンセルしました".to_string();
        }
    }
}

fn truncate_path_left(s: &str, max_cols: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let total_w: usize = s.chars().map(|c| c.width().unwrap_or(1)).sum();
    if total_w <= max_cols {
        return s.to_string();
    }
    let budget = max_cols.saturating_sub(3);
    let mut kept: Vec<char> = Vec::new();
    let mut acc = 0usize;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(1);
        if acc + cw > budget {
            break;
        }
        acc += cw;
        kept.push(c);
    }
    kept.reverse();
    let tail: String = kept.into_iter().collect();
    format!("...{tail}")
}

fn truncate_display_right(s: &str, max_cols: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let total_w: usize = s.chars().map(|c| c.width().unwrap_or(1)).sum();
    if total_w <= max_cols {
        return s.to_string();
    }
    let budget = max_cols.saturating_sub(2);
    let mut out = String::new();
    let mut acc = 0usize;
    for c in s.chars() {
        let cw = c.width().unwrap_or(1);
        if acc + cw > budget {
            break;
        }
        acc += cw;
        out.push(c);
    }
    out.push('…');
    out
}

impl eframe::App for LzEditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_dialog();
        self.handle_events(ctx);

        self.render_status_bar(ctx);
        self.render_buffer_strip(ctx);
        self.render_fkey_bar(ctx);

        match self.mode {
            AppMode::Editor => self.render_editor_canvas(ctx),
            AppMode::Filer => self.render_filer_canvas(ctx),
        }

        self.render_help_overlay(ctx);
        self.render_filer_modal(ctx);
        self.render_inline_path_prompt(ctx);

        if self.pending_dialog.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
