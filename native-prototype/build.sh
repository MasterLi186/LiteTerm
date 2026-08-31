#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
cd -- "$SCRIPT_DIR"

die() {
    printf '错误：%s\n' "$*" >&2
    exit 1
}

if (( $# != 0 )); then
    die "build.sh 不接受参数"
fi

REQUIRED_PACKAGER_VERSION="0.11.8"
CARGO_TOML="$SCRIPT_DIR/Cargo.toml"
DIST_DIR="$SCRIPT_DIR/dist/installers"

if [[ ! -f "$CARGO_TOML" ]]; then
    die "未找到 Cargo.toml：$CARGO_TOML"
fi

PACKAGE_VERSION="$(awk '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { if (in_package) exit }
    in_package && /^[[:space:]]*version[[:space:]]*=/ {
        value = $0
        sub(/^[^"]*"/, "", value)
        sub(/".*$/, "", value)
        print value
        exit
    }
' "$CARGO_TOML")"
[[ -n "$PACKAGE_VERSION" ]] || die "无法从 Cargo.toml 读取 [package] version"

command -v cargo >/dev/null 2>&1 || die "未找到 cargo"
command -v dpkg >/dev/null 2>&1 || die "未找到 dpkg"
command -v dpkg-deb >/dev/null 2>&1 || die "未找到 dpkg-deb"

DEB_ARCH="$(dpkg --print-architecture)"
[[ -n "$DEB_ARCH" ]] || die "无法读取 Debian 架构"

PACKAGER_VERSION="$(cargo packager --version 2>/dev/null || true)"
if [[ "$PACKAGER_VERSION" != "cargo-packager $REQUIRED_PACKAGER_VERSION" ]]; then
    echo "安装 cargo-packager $REQUIRED_PACKAGER_VERSION..."
    cargo install cargo-packager --version "$REQUIRED_PACKAGER_VERSION" --locked --force
    PACKAGER_VERSION="$(cargo packager --version 2>/dev/null || true)"
fi
[[ "$PACKAGER_VERSION" == "cargo-packager $REQUIRED_PACKAGER_VERSION" ]] || \
    die "cargo-packager 版本不符合要求（需要 $REQUIRED_PACKAGER_VERSION，实际：${PACKAGER_VERSION:-缺失}）"

echo "=== Native release 编译 ==="
cargo build --release --locked --bin liteterm-native

BINARY="$SCRIPT_DIR/target/release/liteterm-native"
[[ -x "$BINARY" ]] || die "未找到 Native release 构建产物：$BINARY"

# Only remove the exact installer directory owned by this script. Refuse to
# operate on symlinks or unexpected path shapes before invoking rm.
EXPECTED_DIST_DIR="$SCRIPT_DIR/dist/installers"
[[ "$DIST_DIR" == "$EXPECTED_DIST_DIR" ]] || die "安装包输出路径保护校验失败：$DIST_DIR"
[[ ! -L "$SCRIPT_DIR/dist" ]] || die "拒绝操作符号链接目录：$SCRIPT_DIR/dist"
[[ ! -L "$DIST_DIR" ]] || die "拒绝操作符号链接目录：$DIST_DIR"
if [[ -e "$DIST_DIR" && ! -d "$DIST_DIR" ]]; then
    die "安装包输出路径不是目录：$DIST_DIR"
fi
if [[ -d "$DIST_DIR" ]]; then
    rm -rf -- "$DIST_DIR"
fi
mkdir -p -- "$DIST_DIR"

echo "=== Debian 安装包生成 ==="
cargo packager --release --formats deb --verbose

mapfile -t DEB_CANDIDATES < <(find "$DIST_DIR" -maxdepth 1 -type f -name '*.deb' -print)
[[ "${#DEB_CANDIDATES[@]}" -eq 1 ]] || \
    die "cargo-packager 应生成且仅生成一个 .deb，实际 ${#DEB_CANDIDATES[@]} 个"

PACKAGE="$DIST_DIR/LiteTerm-Native_${PACKAGE_VERSION}_${DEB_ARCH}.deb"
if [[ "${DEB_CANDIDATES[0]}" != "$PACKAGE" ]]; then
    mv -- "${DEB_CANDIDATES[0]}" "$PACKAGE"
fi

mapfile -t OUTPUTS < <(find "$DIST_DIR" -maxdepth 1 -type f -print)
[[ "${#OUTPUTS[@]}" -eq 1 && "${OUTPUTS[0]}" == "$PACKAGE" ]] || \
    die "安装包输出目录中存在非预期文件"

CONTROL_PACKAGE="$(dpkg-deb -f "$PACKAGE" Package)" || die "无法读取 Debian Package 字段"
CONTROL_VERSION="$(dpkg-deb -f "$PACKAGE" Version)" || die "无法读取 Debian Version 字段"
CONTROL_ARCH="$(dpkg-deb -f "$PACKAGE" Architecture)" || die "无法读取 Debian Architecture 字段"
CONTROL_DEPENDS="$(dpkg-deb -f "$PACKAGE" Depends)" || die "无法读取 Debian Depends 字段"

[[ "$CONTROL_PACKAGE" == "liteterm-native" ]] || die "Package 字段错误：$CONTROL_PACKAGE"
[[ "$CONTROL_VERSION" == "$PACKAGE_VERSION" ]] || die "Version 字段错误：$CONTROL_VERSION"
[[ "$CONTROL_ARCH" == "$DEB_ARCH" ]] || die "Architecture 字段错误：$CONTROL_ARCH"

for dependency in libc6 libgcc-s1 libudev1 zlib1g; do
    if ! awk -F',' -v expected="$dependency" '
        {
            for (i = 1; i <= NF; i++) {
                item = $i
                sub(/^[[:space:]]+/, "", item)
                sub(/[[:space:]]+$/, "", item)
                sub(/[[:space:]]*\(.*/, "", item)
                if (item == expected) found = 1
            }
        }
        END { exit(found ? 0 : 1) }
    ' <<< "$CONTROL_DEPENDS"; then
        die "Depends 缺少 $dependency：$CONTROL_DEPENDS"
    fi
done

VERIFY_DIR="$(mktemp -d "${TMPDIR:-/tmp}/liteterm-native-deb.XXXXXX")"
trap 'rm -rf -- "$VERIFY_DIR"' EXIT
dpkg-deb -x "$PACKAGE" "$VERIFY_DIR"

PACKAGE_BINARY="$VERIFY_DIR/usr/bin/liteterm-native"
[[ -x "$PACKAGE_BINARY" ]] || die "包内缺少可执行文件 /usr/bin/liteterm-native"

DESKTOP_FILE="$VERIFY_DIR/usr/share/applications/liteterm-native.desktop"
[[ -f "$DESKTOP_FILE" ]] || die "包内缺少 liteterm-native.desktop"
grep -Fxq 'Exec=liteterm-native' "$DESKTOP_FILE" || die "desktop 入口缺少 Exec=liteterm-native"
grep -Fxq 'Icon=liteterm-native' "$DESKTOP_FILE" || die "desktop 入口缺少 Icon=liteterm-native"

for size in 32x32 128x128 256x256 512x512 1024x1024; do
    icon="$VERIFY_DIR/usr/share/icons/hicolor/$size/apps/liteterm-native.png"
    [[ -f "$icon" ]] || die "包内缺少 $size 图标"
done

ABS_PACKAGE="$(realpath -- "$PACKAGE")"
REL_PACKAGE="${PACKAGE#"$SCRIPT_DIR/"}"
printf 'Debian 安装包：%s\n' "$ABS_PACKAGE"
printf '可复制安装命令（在 %s 执行）：sudo apt install ./%s\n' "$SCRIPT_DIR" "$REL_PACKAGE"
