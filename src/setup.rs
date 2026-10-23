//! Правило sudoers, чтобы переключать режим крышки без пароля.
//!
//! Разрешаются ровно две команды: `pmset -a disablesleep 0` и `pmset -a disablesleep 1`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Command;

pub const RULE_PATH: &str = "/etc/sudoers.d/keep-awake";
/// Файлы с точкой в имени sudo пропускает, поэтому черновик не вступит в силу раньше проверки.
const DRAFT_PATH: &str = "/etc/sudoers.d/.keep-awake.draft";

pub fn is_installed() -> bool {
    Path::new(RULE_PATH).exists()
}

/// Вызывать от root.
pub fn install(user: &str) -> Result<(), String> {
    let valid = !user.is_empty()
        && !user.starts_with('-')
        && user.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !valid {
        return Err(format!("странное имя пользователя: {user:?}"));
    }

    let rule = format!(
        "# Added by keep-awake: toggle sleep without a password.\n\
         {user} ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 0, /usr/bin/pmset -a disablesleep 1\n"
    );
    let _ = fs::remove_file(DRAFT_PATH);
    let mut draft = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o440)
        .open(DRAFT_PATH)
        .map_err(|e| format!("не создать {DRAFT_PATH}: {e}"))?;
    draft
        .write_all(rule.as_bytes())
        .map_err(|e| format!("не записать {DRAFT_PATH}: {e}"))?;
    drop(draft);

    let checked = Command::new("/usr/sbin/visudo").args(["-cqf", DRAFT_PATH]).status();
    if !matches!(checked, Ok(s) if s.success()) {
        let _ = fs::remove_file(DRAFT_PATH);
        return Err("visudo забраковал правило, ничего не изменено".into());
    }
    fs::rename(DRAFT_PATH, RULE_PATH).map_err(|e| format!("не переименовать в {RULE_PATH}: {e}"))
}

/// Вызывать от root.
pub fn remove() -> Result<(), String> {
    match fs::remove_file(RULE_PATH) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("не удалить {RULE_PATH}: {e}")),
    }
}
