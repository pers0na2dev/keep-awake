# keep-awake

Закрытый MacBook без внешнего монитора и зарядки засыпает, и `caffeinate` от этого не спасает. keep-awake запрещает сон полностью, в том числе с закрытой крышкой. Его можно запускать из терминала или держать иконкой в менюбаре.

Нужна macOS 11 или новее. Сборки универсальные, для Apple Silicon и Intel.

## Установка

Приложение с иконкой в менюбаре:

```bash
curl -fsSL -o /tmp/KeepAwake.zip https://github.com/pers0na2dev/keep-awake/releases/latest/download/KeepAwake-macos-universal.zip
ditto -x -k /tmp/KeepAwake.zip /Applications && open /Applications/KeepAwake.app
```

Можно и руками: скачать zip со страницы [Releases](https://github.com/pers0na2dev/keep-awake/releases/latest) и перетащить `KeepAwake.app` в «Программы». Чтобы иконка появлялась после перезагрузки, добавь KeepAwake в «Системные настройки → Основные → Объекты входа».

Только CLI:

```bash
curl -fsSL https://github.com/pers0na2dev/keep-awake/releases/latest/download/keep-awake-macos-universal.tar.gz | tar xz
sudo mkdir -p /usr/local/bin && sudo mv keep-awake /usr/local/bin/
```

Если приложение уже стоит, качать CLI не нужно, внутри тот же бинарник:

```bash
sudo mkdir -p /usr/local/bin
sudo ln -sf /Applications/KeepAwake.app/Contents/MacOS/KeepAwake /usr/local/bin/keep-awake
```

Для сборки из исходников нужен Rust 1.89 или новее: `cargo install --git https://github.com/pers0na2dev/keep-awake`.

Сборки из Releases подписаны Developer ID и нотаризованы, Gatekeeper пропускает их и после скачивания через браузер. `spctl -a -vv /Applications/KeepAwake.app` должен ответить `source=Notarized Developer ID`.

## Менюбар

В меню у чашки:

- «Включить» и «Выключить». Залитая чашка значит, что keep-awake включён.
- «Не гасить экран».
- «Не спать с закрытой крышкой». Для этого режима нужен пароль администратора, подробности ниже.
- «Не спрашивать пароль…» спросит его один раз, после этого крышка переключается без пароля.
- «Выйти» сначала возвращает сон, потом закрывает приложение.

Из терминала иконку запускает `keep-awake menubar`. Она отцепляется от терминала, окно можно закрыть.

## Терминал

```bash
keep-awake                        # не спать совсем, в том числе с закрытой крышкой; Ctrl+C — выход
keep-awake -t 1h30m               # то же на полтора часа (понимает 45s, 30m, 2h, 1d)
keep-awake --allow-display-sleep  # экран гаснет, Mac работает
keep-awake --no-lid               # без пароля, но закрытая крышка усыпит Mac
keep-awake menubar                # иконка в менюбаре
keep-awake setup                  # не спрашивать пароль для режима крышки
keep-awake setup --remove         # снова спрашивать
keep-awake --reset                # вернуть обычный сон после аварийного завершения
```

## Закрытая крышка и пароль

Сон по простою и гашение экрана запрещаются IOKit-ассертами `PreventUserIdleSystemSleep`, `PreventSystemSleep` и `PreventUserIdleDisplaySleep`, как у `caffeinate -dims`. Прав на это не нужно. Ассерты видно в `pmset -g assertions` под именем `keep-awake`, а если процесс умрёт, ядро снимет их само.

С крышкой так не выйдет. Нужен `pmset -a disablesleep 1`, а его может менять только root. CLI ради этого перезапускается через `sudo`. Иконка спрашивает пароль системным окном, и при включении, и при выключении.

keep-awake запоминает, какое значение `disablesleep` стояло до запуска, и возвращает его, когда его останавливают: по Ctrl+C, при закрытии терминала, по `kill`, через «Выйти» в меню, при выходе из системы. Если сон был запрещён ещё до запуска, keep-awake его не трогает. Включённым может быть только один экземпляр, иначе первый при выходе вернул бы сон, пока второй ещё работает. Второй пишет «keep-awake уже включён в другом окне терминала или в менюбаре» и не запускается.

Чтобы пароль не спрашивали, выполни `keep-awake setup` или выбери «Не спрашивать пароль…». Это кладёт в `/etc/sudoers.d/keep-awake` правило, проверенное через `visudo`: твой пользователь может без пароля запускать ровно две команды.

```
/usr/bin/pmset -a disablesleep 0
/usr/bin/pmset -a disablesleep 1
```

Убрать правило можно командой `keep-awake setup --remove`.

С закрытой крышкой Mac работает по-настоящему. В рюкзаке или чехле ему негде остывать.

## Если Mac перестал засыпать

После `kill -9` или отключения питания keep-awake не успевает вернуть настройку, и `disablesleep` остаётся равным 1. Проверить можно так: `pmset -g | grep SleepDisabled`. Если строки нет или там 0, всё в порядке. Вернуть сон:

```bash
keep-awake --reset
```

Если иконку запускали командой `keep-awake menubar`, её лог лежит в `~/Library/Logs/keep-awake.log`.

## Удаление

```bash
keep-awake setup --remove                  # если ставил правило без пароля
rm -rf /Applications/KeepAwake.app
sudo rm -f /usr/local/bin/keep-awake
rm -f ~/Library/Logs/keep-awake.log
```

## Сборка релиза

`scripts/build-release.sh` собирает универсальный бинарник (arm64 и x86_64), кладёт его в `KeepAwake.app`, подписывает и складывает в `dist/` три файла: `KeepAwake-macos-universal.zip`, `keep-awake-macos-universal.tar.gz` и `SHA256SUMS.txt`.

Подписывает он первым сертификатом «Developer ID Application» из связки ключей, а если такого нет, ad-hoc. Другой сертификат задаётся через `SIGN_IDENTITY` или `APPLE_SIGNING_IDENTITY`.

Нотаризация включается, когда есть учётные данные Apple. Их можно один раз сохранить в связку ключей. Команда спрашивает пароль, поэтому запускай её в терминале:

```bash
xcrun notarytool store-credentials keep-awake-notary --apple-id you@example.com --team-id TEAMID
NOTARY_PROFILE=keep-awake-notary scripts/build-release.sh
```

Или передать через окружение, например в CI, только не коммить их:

```bash
APPLE_ID=you@example.com APPLE_PASSWORD=xxxx-xxxx-xxxx-xxxx APPLE_TEAM_ID=TEAMID scripts/build-release.sh
```

Пароль приложения создаётся на [appleid.apple.com](https://appleid.apple.com), в разделе «Вход и безопасность → Пароли приложений». Иконку приложения рисует `swift scripts/make-icon.swift`.
