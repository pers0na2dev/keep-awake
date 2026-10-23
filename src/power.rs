//! Всё, что держит Mac бодрствующим.
//!
//! * IOPM-ассерты (те же, что у `caffeinate -dims`) запрещают сон по простою и гашение экрана.
//!   Они не требуют прав и снимаются ядром сами, если процесс умер.
//! * `pmset -a disablesleep 1` запрещает сон совсем, в том числе при закрытой крышке.
//!   Это системная настройка: нужен root, а после выхода её надо вернуть.

use std::ffi::{CString, c_char, c_void};
use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Command, Stdio};

/// Как получать root для `pmset`.
#[derive(Clone, Copy)]
pub enum Elevation {
    /// Только без пароля: мы root или есть правило от `keep-awake setup`.
    Silent,
    /// Если без пароля нельзя, спросить пароль системным окном.
    Prompt,
}

pub enum Error {
    /// Без пароля нельзя, а спрашивать его не разрешили.
    NeedsPassword,
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsPassword => f.write_str("для режима закрытой крышки нужны права администратора"),
            Self::Failed(e) => f.write_str(e),
        }
    }
}

/// Держит Mac бодрствующим, пока жив. Части можно включать и выключать на ходу.
pub struct KeepAwake {
    _idle: [Assertion; 2],
    display: Option<Assertion>,
    lid: Option<LidGuard>,
    /// Последним полем: отпускается уже после того, как крышка вернулась в прежний режим.
    _lock: File,
}

impl KeepAwake {
    /// Сначала крышка: если на неё не хватит прав, ассерты даже не создаются.
    pub fn start(display: bool, lid: Option<Elevation>) -> Result<Self, Error> {
        let lock = take_lock()?;
        let lid = lid.map(LidGuard::engage).transpose()?;
        let idle = [
            Assertion::new("PreventUserIdleSystemSleep")?,
            Assertion::new("PreventSystemSleep")?,
        ];
        let mut this = Self { _idle: idle, display: None, lid, _lock: lock };
        this.set_display(display)?;
        Ok(this)
    }

    pub fn set_display(&mut self, on: bool) -> Result<(), Error> {
        match (on, self.display.is_some()) {
            (true, false) => self.display = Some(Assertion::new("PreventUserIdleDisplaySleep")?),
            (false, true) => self.display = None,
            _ => {}
        }
        Ok(())
    }

    /// `Some` — не спать с закрытой крышкой, `None` — вернуть прежнее поведение.
    /// Если вернуть не вышло (например, пароль не ввели), режим остаётся включённым.
    pub fn set_lid(&mut self, lid: Option<Elevation>) -> Result<(), Error> {
        match (lid, &mut self.lid) {
            (Some(elevation), None) => self.lid = Some(LidGuard::engage(elevation)?),
            (None, Some(guard)) => {
                guard.release()?;
                self.lid = None;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Включённым может быть только один keep-awake: иначе первый при выходе вернёт сон, пока второй ещё «держит».
/// Файл общий для пользователя и root, поэтому лежит в /tmp; flock отпускается и при аварийном выходе.
fn take_lock() -> Result<File, Error> {
    const LOCK_PATH: &str = "/tmp/keep-awake.lock";
    let file = match File::open(LOCK_PATH) {
        Err(e) if e.kind() == ErrorKind::NotFound => OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o644)
            .open(LOCK_PATH),
        other => other,
    }
    .map_err(|e| Error::Failed(format!("не открыть {LOCK_PATH}: {e}")))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(Error::Failed(
            "keep-awake уже включён в другом окне терминала или в менюбаре".into(),
        )),
        Err(TryLockError::Error(e)) => Err(Error::Failed(format!("не заблокировать {LOCK_PATH}: {e}"))),
    }
}

// ---------- закрытая крышка: pmset disablesleep ----------

const PMSET: &str = "/usr/bin/pmset";

/// Пока активен, держит `disablesleep = 1`; потом возвращает то, что было до него.
struct LidGuard {
    elevation: Elevation,
    /// `false`, если сон уже был запрещён до нас: тогда и возвращать нечего.
    active: bool,
}

impl LidGuard {
    fn engage(elevation: Elevation) -> Result<Self, Error> {
        if sleep_disabled()? {
            eprintln!(
                "Сон уже был запрещён до запуска (disablesleep = 1), при выходе оставлю как есть.\n\
                 Если это след аварийно завершённого keep-awake, выполни: keep-awake --reset"
            );
            return Ok(Self { elevation, active: false });
        }
        set_sleep_disabled(true, elevation)?;
        if !sleep_disabled()? {
            eprintln!("Предупреждение: pmset не подтвердил disablesleep = 1, крышка может усыпить Mac.");
        }
        Ok(Self { elevation, active: true })
    }

    fn release(&mut self) -> Result<(), Error> {
        if self.active {
            set_sleep_disabled(false, self.elevation)?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for LidGuard {
    fn drop(&mut self) {
        if let Err(e) = self.release() {
            eprintln!("keep-awake: {e}\nВерни сон вручную: keep-awake --reset");
        }
    }
}

/// `pmset -g` выводит строку `SleepDisabled 1`, когда сон запрещён.
pub fn sleep_disabled() -> Result<bool, Error> {
    let out = Command::new(PMSET)
        .arg("-g")
        .output()
        .map_err(|e| Error::Failed(format!("не запустился pmset: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().any(|line| {
        let mut words = line.split_whitespace();
        words.next() == Some("SleepDisabled") && words.next() == Some("1")
    }))
}

pub fn set_sleep_disabled(on: bool, elevation: Elevation) -> Result<(), Error> {
    let value = if on { "1" } else { "0" };
    let failed = |e: String| Error::Failed(format!("pmset -a disablesleep {value}: {e}"));

    if is_root() {
        let status = Command::new(PMSET)
            .args(["-a", "disablesleep", value])
            .status()
            .map_err(|e| failed(e.to_string()))?;
        return if status.success() { Ok(()) } else { Err(failed(status.to_string())) };
    }

    // Без пароля это пройдёт, если есть правило от `keep-awake setup` или свежий тикет sudo.
    let sudo = Command::new("/usr/bin/sudo")
        .args(["-n", PMSET, "-a", "disablesleep", value])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if matches!(sudo, Ok(s) if s.success()) {
        return Ok(());
    }

    match elevation {
        Elevation::Silent => Err(Error::NeedsPassword),
        Elevation::Prompt => {
            let why = if on {
                "keep-awake хочет запретить сон при закрытой крышке."
            } else {
                "keep-awake хочет вернуть обычный сон."
            };
            run_as_admin(&format!("{PMSET} -a disablesleep {value}"), why).map_err(failed)
        }
    }
}

/// Запускает shell-команду от root, спросив пароль системным окном.
pub fn run_as_admin(shell_command: &str, prompt: &str) -> Result<(), String> {
    let script = format!(
        "do shell script {} with prompt {} with administrator privileges",
        applescript_string(shell_command),
        applescript_string(prompt),
    );
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("не запустился osascript: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("(-128)") {
        Err("пароль не введён".into())
    } else {
        Err(stderr.trim().to_string())
    }
}

fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn is_root() -> bool {
    unsafe { geteuid() == 0 }
}

// ---------- сон по простою и экран: IOPM-ассерты ----------

type CFStringRef = *const c_void;
type IOPMAssertionID = u32;
type IOReturn = i32;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;

unsafe extern "C" {
    fn geteuid() -> u32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> CFStringRef;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        level: u32,
        name: CFStringRef,
        id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(id: IOPMAssertionID) -> IOReturn;
}

struct CfString(CFStringRef);

impl CfString {
    fn new(s: &str) -> Self {
        let c = CString::new(s).expect("строка без нулевых байтов");
        let r = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8) };
        assert!(!r.is_null(), "CFStringCreateWithCString вернул NULL");
        Self(r)
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

/// Ассерт питания; снимается при drop (и ядром автоматически, если процесс умер).
struct Assertion(IOPMAssertionID);

impl Assertion {
    fn new(kind: &str) -> Result<Self, Error> {
        let kind_cf = CfString::new(kind);
        let name = CfString::new("keep-awake");
        let mut id = 0;
        let ret = unsafe { IOPMAssertionCreateWithName(kind_cf.0, K_IOPM_ASSERTION_LEVEL_ON, name.0, &mut id) };
        if ret != 0 {
            return Err(Error::Failed(format!("IOKit не выдал ассерт {kind}: 0x{ret:08x}")));
        }
        Ok(Self(id))
    }
}

impl Drop for Assertion {
    fn drop(&mut self) {
        unsafe { IOPMAssertionRelease(self.0) };
    }
}
