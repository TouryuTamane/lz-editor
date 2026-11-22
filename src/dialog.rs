use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;

#[derive(Debug, Clone)]
pub enum DialogResult {
    SavePath(Option<PathBuf>),
    OpenPath(Option<PathBuf>),
    /// Neither zenity nor kdialog is installed; trigger built-in inline path prompt.
    Unavailable { is_save: bool },
}

/// Spawn a non-blocking native file save dialog using `zenity` (GNOME/GTK) or `kdialog`.
pub fn spawn_save_dialog(initial_path: Option<&Path>) -> Receiver<DialogResult> {
    let initial = initial_path.map(|p| p.to_path_buf());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let res = run_save_dialog(initial.as_deref());
        let _ = tx.send(res);
    });
    rx
}

/// Spawn a non-blocking native file open dialog using `zenity` (GNOME/GTK) or `kdialog`.
pub fn spawn_open_dialog(initial_dir: Option<&Path>) -> Receiver<DialogResult> {
    let initial = initial_dir.map(|p| p.to_path_buf());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let res = run_open_dialog(initial.as_deref());
        let _ = tx.send(res);
    });
    rx
}

fn run_save_dialog(initial_path: Option<&Path>) -> DialogResult {
    let mut cmd = Command::new("zenity");
    cmd.args([
        "--file-selection",
        "--save",
        "--confirm-overwrite",
        "--title=LZ Editor - 名前を付けて保存",
    ]);
    if let Some(p) = initial_path {
        cmd.arg(format!("--filename={}", p.display()));
    } else if let Ok(cwd) = std::env::current_dir() {
        cmd.arg(format!("--filename={}/untitled.txt", cwd.display()));
    }

    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return DialogResult::SavePath(if s.is_empty() { None } else { Some(PathBuf::from(s)) });
        } else {
            return DialogResult::SavePath(None);
        }
    }

    // Fallback: kdialog
    let mut kcmd = Command::new("kdialog");
    kcmd.args(["--getsavefilename"]);
    if let Some(p) = initial_path {
        kcmd.arg(p);
    } else {
        kcmd.arg(".");
    }
    if let Ok(output) = kcmd.output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return DialogResult::SavePath(if s.is_empty() { None } else { Some(PathBuf::from(s)) });
        } else {
            return DialogResult::SavePath(None);
        }
    }

    DialogResult::Unavailable { is_save: true }
}

fn run_open_dialog(initial_dir: Option<&Path>) -> DialogResult {
    let mut cmd = Command::new("zenity");
    cmd.args([
        "--file-selection",
        "--title=LZ Editor - ファイルを開く",
    ]);
    if let Some(p) = initial_dir {
        cmd.arg(format!("--filename={}/", p.display()));
    }

    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return DialogResult::OpenPath(if s.is_empty() { None } else { Some(PathBuf::from(s)) });
        } else {
            return DialogResult::OpenPath(None);
        }
    }

    let mut kcmd = Command::new("kdialog");
    kcmd.args(["--getopenfilename"]);
    if let Some(p) = initial_dir {
        kcmd.arg(p);
    } else {
        kcmd.arg(".");
    }
    if let Ok(output) = kcmd.output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return DialogResult::OpenPath(if s.is_empty() { None } else { Some(PathBuf::from(s)) });
        } else {
            return DialogResult::OpenPath(None);
        }
    }

    DialogResult::Unavailable { is_save: false }
}
