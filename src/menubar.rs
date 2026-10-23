//! Иконка в менюбаре: включить/выключить, не гасить экран, не спать с закрытой крышкой.

use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSControlStateValueOff,
    NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString, ns_string};

use crate::power::{self, Elevation, KeepAwake};
use crate::setup;

/// Общий с обработчиком сигналов: по SIGTERM/SIGINT/SIGHUP он снимает всё и выходит.
static AWAKE: Mutex<Option<KeepAwake>> = Mutex::new(None);

fn awake() -> MutexGuard<'static, Option<KeepAwake>> {
    AWAKE.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "C" {
    fn setsid() -> i32;
}

/// Запускает иконку. Без `foreground` отцепляется от терминала и сразу возвращается.
pub fn launch(foreground: bool) -> Result<(), String> {
    let lock_path = std::env::temp_dir().join("keep-awake-menubar.lock");
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("не открыть {}: {e}", lock_path.display()))?;
    if lock.try_lock().is_err() {
        println!("Иконка keep-awake уже в менюбаре.");
        return Ok(());
    }
    if foreground {
        return run_app(lock);
    }
    drop(lock);
    detach()
}

fn detach() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("не найден путь к себе: {e}"))?;
    let log_path = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Logs/keep-awake.log"))
        .unwrap_or_else(|| std::env::temp_dir().join("keep-awake.log"));
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("не открыть {}: {e}", log_path.display()))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = Command::new(exe);
    cmd.args(["menubar", "--foreground"])
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    // Своя сессия: закрытие терминала не пришлёт SIGHUP.
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| format!("не запустился фоновый процесс: {e}"))?;
    println!("Иконка keep-awake появилась в менюбаре. Лог: {}", log_path.display());
    Ok(())
}

fn run_app(_lock: File) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("менюбар запускается только из главного потока")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let controller = Controller::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));

    ctrlc::set_handler(|| {
        awake().take();
        std::process::exit(0);
    })
    .map_err(|e| format!("не удалось перехватить сигналы: {e}"))?;

    app.run();
    Ok(())
}

struct Ivars {
    status_item: Retained<NSStatusItem>,
    status_line: Retained<NSMenuItem>,
    toggle: Retained<NSMenuItem>,
    display: Retained<NSMenuItem>,
    lid: Retained<NSMenuItem>,
    setup: Retained<NSMenuItem>,
    keep_display: Cell<bool>,
    keep_lid: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject можно наследовать, Drop не переопределён.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KeepAwakeController"]
    #[ivars = Ivars]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSApplicationDelegate for Controller {
        // Выход из системы, перезагрузка: вернуть сон до того, как процесс умрёт.
        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            awake().take();
        }
    }

    impl Controller {
        #[unsafe(method(toggle:))]
        fn toggle(&self, _sender: Option<&AnyObject>) {
            if awake().is_some() {
                self.stop();
            } else {
                self.start();
            }
            self.refresh();
        }

        #[unsafe(method(toggleDisplay:))]
        fn toggle_display(&self, _sender: Option<&AnyObject>) {
            let want = !self.ivars().keep_display.get();
            let result = awake().as_mut().map(|k| k.set_display(want));
            match result {
                Some(Err(e)) => self.alert("Не удалось переключить экран", &e.to_string()),
                _ => self.ivars().keep_display.set(want),
            }
            self.refresh();
        }

        #[unsafe(method(toggleLid:))]
        fn toggle_lid(&self, _sender: Option<&AnyObject>) {
            let want = !self.ivars().keep_lid.get();
            let result = awake().as_mut().map(|k| k.set_lid(want.then_some(Elevation::Prompt)));
            match result {
                Some(Err(e)) => self.alert("Не удалось переключить режим крышки", &e.to_string()),
                _ => self.ivars().keep_lid.set(want),
            }
            self.refresh();
        }

        #[unsafe(method(setupPasswordless:))]
        fn setup_passwordless(&self, _sender: Option<&AnyObject>) {
            if let Err(e) = install_rule() {
                self.alert("Не удалось отключить пароль", &e);
            }
            self.refresh();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            if self.stop() {
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            }
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let item = |title: &str, action: Option<Sel>| unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                action,
                ns_string!(""),
            )
        };
        let status_line = item("", None);
        let toggle = item("Включить", Some(sel!(toggle:)));
        let display = item("Не гасить экран", Some(sel!(toggleDisplay:)));
        let lid = item("Не спать с закрытой крышкой", Some(sel!(toggleLid:)));
        let setup = item("Не спрашивать пароль…", Some(sel!(setupPasswordless:)));
        let quit = item("Выйти", Some(sel!(quit:)));

        let menu = NSMenu::new(mtm);
        menu.addItem(&status_line);
        menu.addItem(&toggle);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&display);
        menu.addItem(&lid);
        menu.addItem(&setup);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&quit);

        let status_item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        status_item.setMenu(Some(&menu));

        let this = Self::alloc(mtm).set_ivars(Ivars {
            status_item,
            status_line,
            toggle: toggle.clone(),
            display: display.clone(),
            lid: lid.clone(),
            setup: setup.clone(),
            keep_display: Cell::new(true),
            keep_lid: Cell::new(true),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };

        let target: &AnyObject = &this;
        for item in [&toggle, &display, &lid, &setup, &quit] {
            // SAFETY: контроллер живёт до конца app.run(), дольше пунктов меню.
            unsafe { item.setTarget(Some(target)) };
        }
        this.refresh();
        this
    }

    fn start(&self) {
        let ivars = self.ivars();
        let lid = ivars.keep_lid.get().then_some(Elevation::Prompt);
        match KeepAwake::start(ivars.keep_display.get(), lid) {
            Ok(k) => *awake() = Some(k),
            Err(e) => self.alert("Не удалось включить keep-awake", &e.to_string()),
        }
    }

    /// `false`, если вернуть сон не вышло: тогда keep-awake остаётся включённым.
    fn stop(&self) -> bool {
        let mut guard = awake();
        let Some(k) = guard.as_mut() else { return true };
        match k.set_lid(None) {
            Ok(()) => {
                *guard = None;
                true
            }
            Err(e) => {
                drop(guard);
                self.alert(
                    "Сон всё ещё запрещён",
                    &format!("{e}. Mac не уснёт и с закрытой крышкой, пока keep-awake включён."),
                );
                false
            }
        }
    }

    fn refresh(&self) {
        let ivars = self.ivars();
        let on = awake().is_some();
        let (symbol, line) = match (on, ivars.keep_lid.get()) {
            (true, true) => ("cup.and.saucer.fill", "Mac не спит, даже с закрытой крышкой"),
            (true, false) => ("cup.and.saucer.fill", "Mac не спит, но крышка его усыпит"),
            (false, _) => ("cup.and.saucer", "Mac спит как обычно"),
        };
        if let Some(button) = ivars.status_item.button(self.mtm()) {
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                Some(ns_string!("keep-awake")),
            );
            match image {
                Some(image) => {
                    image.setTemplate(true);
                    button.setImage(Some(&image));
                }
                None => button.setTitle(&NSString::from_str(if on { "☕︎" } else { "z" })),
            }
            button.setToolTip(Some(&NSString::from_str(line)));
        }
        ivars.status_line.setTitle(&NSString::from_str(line));
        ivars
            .toggle
            .setTitle(&NSString::from_str(if on { "Выключить" } else { "Включить" }));
        let state = |on: bool| if on { NSControlStateValueOn } else { NSControlStateValueOff };
        ivars.display.setState(state(ivars.keep_display.get()));
        ivars.lid.setState(state(ivars.keep_lid.get()));
        ivars.setup.setHidden(setup::is_installed());
    }

    fn alert(&self, title: &str, text: &str) {
        let app = NSApplication::sharedApplication(self.mtm());
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(text));
        alert.runModal();
    }
}

/// Один раз спрашивает пароль и ставит правило sudoers через `keep-awake setup`.
fn install_rule() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("не найден путь к себе: {e}"))?;
    let out = Command::new("/usr/bin/id")
        .arg("-un")
        .output()
        .map_err(|e| format!("не узнать имя пользователя: {e}"))?;
    let user = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let command = format!(
        "{} setup --user {}",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(&user)
    );
    power::run_as_admin(&command, "keep-awake хочет переключать режим крышки без пароля.")
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
