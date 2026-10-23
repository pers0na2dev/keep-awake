//! keep-awake — не даёт Mac уснуть: ни по простою, ни из меню, ни при закрытой крышке.

mod menubar;
mod power;
mod setup;

use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

use power::{Elevation, KeepAwake};

#[derive(Parser)]
#[command(
    name = "keep-awake",
    version,
    about = "Не даёт Mac уснуть, даже с закрытой крышкой",
    after_help = "Режим закрытой крышки меняет системную настройку, поэтому нужен пароль администратора:\n\
                  keep-awake сам перезапустится через sudo. Чтобы не вводить его каждый раз: keep-awake setup\n\
                  Если процесс убили через kill -9, верни обычный сон командой: keep-awake --reset",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    #[command(flatten)]
    run: RunArgs,
}

#[derive(Args)]
struct RunArgs {
    /// Выключиться через это время: 45s, 30m, 2h, 1h30m (без суффикса — секунды)
    #[arg(short, long, value_name = "ВРЕМЯ", value_parser = parse_duration)]
    time: Option<Duration>,

    /// Не трогать режим крышки: пароль не нужен, но с закрытой крышкой Mac уснёт
    #[arg(long)]
    no_lid: bool,

    /// Разрешить экрану гаснуть (система всё равно не уснёт)
    #[arg(long)]
    allow_display_sleep: bool,

    /// Вернуть обычный сон, если keep-awake завершился аварийно
    #[arg(long, conflicts_with_all = ["time", "no_lid", "allow_display_sleep"])]
    reset: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Иконка в менюбаре с включением и выключением
    Menubar {
        /// Не отцепляться от терминала (удобно для отладки)
        #[arg(long)]
        foreground: bool,
    },
    /// Переключать режим крышки без пароля (правило в /etc/sudoers.d/keep-awake)
    Setup {
        /// Удалить правило и снова спрашивать пароль
        #[arg(long)]
        remove: bool,

        /// Кому разрешить; по умолчанию тот, кто вызвал sudo
        #[arg(long, hide = true)]
        user: Option<String>,
    },
}

fn main() -> ExitCode {
    let result = if launched_as_app() {
        menubar::launch(true)
    } else {
        let cli = Cli::parse();
        match cli.command {
            Some(Cmd::Menubar { foreground }) => menubar::launch(foreground),
            Some(Cmd::Setup { remove, user }) => run_setup(remove, user),
            None if cli.run.reset => reset(),
            None => run_cli(cli.run),
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("keep-awake: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Открыли KeepAwake.app (Finder, Dock, объекты входа): сразу иконка в менюбаре.
/// С аргументами или через симлинк вроде /usr/local/bin/keep-awake это обычный CLI.
fn launched_as_app() -> bool {
    std::env::args_os().len() == 1
        && std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}

fn run_cli(args: RunArgs) -> Result<(), String> {
    // Обработчик ставим до изменения настроек, чтобы Ctrl+C в любой момент приводил к откату.
    let (tx, rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .map_err(|e| format!("не удалось перехватить Ctrl+C: {e}"))?;

    let lid = (!args.no_lid).then_some(Elevation::Silent);
    let awake = or_relaunch(
        "Для режима закрытой крышки",
        KeepAwake::start(!args.allow_display_sleep, lid),
    )?;

    println!("keep-awake работает. Ctrl+C — выход.");
    println!("  сон по простою:   запрещён");
    println!(
        "  экран:            {}",
        if args.allow_display_sleep { "может гаснуть" } else { "не гаснет" }
    );
    println!(
        "  закрытая крышка:  {}",
        if args.no_lid { "Mac уснёт (--no-lid)" } else { "Mac не уснёт" }
    );
    match args.time {
        Some(t) => println!("  таймер:           {}", format_duration(t)),
        None => println!("  таймер:           нет"),
    }
    if !args.no_lid {
        println!("\nС закрытой крышкой не убирай ноут в рюкзак: он не спит и будет греться.");
    }

    match args.time {
        Some(t) => {
            if let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(t) {
                println!("\nВремя вышло.");
            }
        }
        None => {
            let _ = rx.recv();
        }
    }

    drop(awake);
    println!("\nГотово, Mac снова может спать.");
    Ok(())
}

fn reset() -> Result<(), String> {
    or_relaunch("Для --reset", power::set_sleep_disabled(false, Elevation::Silent))?;
    println!("Сон снова разрешён (disablesleep = 0).");
    Ok(())
}

fn run_setup(remove: bool, user: Option<String>) -> Result<(), String> {
    if !power::is_root() {
        return Err(relaunch_as_root("Для setup"));
    }
    if remove {
        setup::remove()?;
        println!("Правило удалено, режим крышки снова будет спрашивать пароль.");
        return Ok(());
    }
    let user = user
        .or_else(|| std::env::var("SUDO_USER").ok())
        .ok_or("не понятно, кому разрешить: запусти keep-awake setup без sudo")?;
    setup::install(&user)?;
    println!(
        "Готово: {user} включает режим крышки без пароля. Правило: {}",
        setup::RULE_PATH
    );
    Ok(())
}

/// Если без пароля нельзя, перезапускает keep-awake через sudo с теми же аргументами.
fn or_relaunch<T>(why: &str, result: Result<T, power::Error>) -> Result<T, String> {
    match result {
        Ok(v) => Ok(v),
        Err(power::Error::NeedsPassword) if !power::is_root() => Err(relaunch_as_root(why)),
        Err(e) => Err(e.to_string()),
    }
}

/// Заменяет процесс на `sudo keep-awake …`; возвращается только с ошибкой.
fn relaunch_as_root(why: &str) -> String {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return format!("не найден путь к себе: {e}"),
    };
    eprintln!("{why} нужны права администратора, перезапускаюсь через sudo…");
    let err = Command::new("/usr/bin/sudo")
        .arg("--")
        .arg(exe)
        .args(std::env::args_os().skip(1))
        .exec();
    format!("не удалось запустить sudo: {err}")
}

// ---------- время ----------

fn parse_duration(s: &str) -> Result<Duration, String> {
    let mut total: u64 = 0;
    let mut num = String::new();
    for ch in s.trim().chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        let unit = match ch {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            _ => return Err(format!("непонятная единица «{ch}», нужны s, m, h или d")),
        };
        let n: u64 = num.parse().map_err(|_| format!("перед «{ch}» нет числа"))?;
        total = n
            .checked_mul(unit)
            .and_then(|v| total.checked_add(v))
            .ok_or("слишком большое время")?;
        num.clear();
    }
    if !num.is_empty() {
        let n: u64 = num.parse().map_err(|_| "слишком большое время")?;
        total = total.checked_add(n).ok_or("слишком большое время")?;
    }
    if total == 0 {
        return Err("время должно быть больше нуля".into());
    }
    Ok(Duration::from_secs(total))
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let mut out = String::new();
    if h > 0 {
        out += &format!("{h}h");
    }
    if m > 0 {
        out += &format!("{m}m");
    }
    if s > 0 || out.is_empty() {
        out += &format!("{s}s");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("45"), Ok(Duration::from_secs(45)));
        assert_eq!(parse_duration("30s"), Ok(Duration::from_secs(30)));
        assert_eq!(parse_duration("15m"), Ok(Duration::from_secs(900)));
        assert_eq!(parse_duration("1h30m"), Ok(Duration::from_secs(5400)));
        assert_eq!(parse_duration("1d"), Ok(Duration::from_secs(86_400)));
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("5x").is_err());
        assert!(parse_duration("99999999999999999999h").is_err());
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_secs(5400)), "1h30m");
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(3601)), "1h1s");
    }
}
