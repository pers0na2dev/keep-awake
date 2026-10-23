#!/usr/bin/env bash
# Собирает универсальный (arm64 + x86_64) keep-awake и KeepAwake.app, подписывает и пакует в dist/.
#
#   SIGN_IDENTITY или APPLE_SIGNING_IDENTITY
#                   чем подписывать; по умолчанию первый «Developer ID Application» из связки ключей,
#                   а если его нет — ad-hoc подпись («-»)
#   NOTARY_PROFILE  профиль из `xcrun notarytool store-credentials`
#   APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID
#                   или так: Apple ID, пароль приложения и команда
#   Если задан профиль или все три APPLE_*, сборка нотаризуется.
#
# Запуск из любого места: scripts/build-release.sh
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIST=dist
APP="$DIST/KeepAwake.app"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

IDENTITY=${SIGN_IDENTITY:-${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning |
    sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -1)}}
IDENTITY=${IDENTITY:--}

NOTARIZE=
if [ -n "${NOTARY_PROFILE:-}" ]; then
    NOTARIZE=profile
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    NOTARIZE=env
fi

echo "==> keep-awake $VERSION, подпись: $IDENTITY"

BINS=()
for target in "${TARGETS[@]}"; do
    rustup target add "$target" >/dev/null
    cargo build --release --locked --target "$target"
    BINS+=("target/$target/release/keep-awake")
done

rm -rf "$DIST"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$DIST/keep-awake" "${BINS[@]}"
cp "$DIST/keep-awake" "$APP/Contents/MacOS/KeepAwake"
sed "s/__VERSION__/$VERSION/g" macos/Info.plist > "$APP/Contents/Info.plist"
cp assets/AppIcon.icns "$APP/Contents/Resources/"

sign() {
    if [ "$IDENTITY" = "-" ]; then
        codesign --force --sign - "$@"
    else
        codesign --force --options runtime --timestamp --sign "$IDENTITY" "$@"
    fi
}
sign --identifier io.github.pers0na2dev.keep-awake "$DIST/keep-awake"
sign "$APP"
codesign --verify --strict --verbose=2 "$DIST/keep-awake"
codesign --verify --deep --strict --verbose=2 "$APP"

if [ -n "$NOTARIZE" ]; then
    echo "==> нотаризация"
    STAGE="$DIST/notarize"
    mkdir -p "$STAGE"
    ditto "$APP" "$STAGE/KeepAwake.app"
    cp "$DIST/keep-awake" "$STAGE/"
    ditto -c -k "$STAGE" "$DIST/notarize.zip"
    if [ "$NOTARIZE" = profile ]; then
        xcrun notarytool submit "$DIST/notarize.zip" --keychain-profile "$NOTARY_PROFILE" --wait
    else
        xcrun notarytool submit "$DIST/notarize.zip" \
            --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
    fi
    # Тикет вшивается только в .app; голый бинарник Gatekeeper проверит по сети.
    xcrun stapler staple "$APP"
    rm -rf "$STAGE" "$DIST/notarize.zip"
fi

(
    cd "$DIST"
    ditto -c -k --keepParent KeepAwake.app KeepAwake-macos-universal.zip
    tar -czf keep-awake-macos-universal.tar.gz keep-awake
    shasum -a 256 KeepAwake-macos-universal.zip keep-awake-macos-universal.tar.gz > SHA256SUMS.txt
)

echo "==> готово:"
ls -lh "$DIST"/*.zip "$DIST"/*.tar.gz "$DIST"/SHA256SUMS.txt
