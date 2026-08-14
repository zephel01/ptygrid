#!/bin/sh
# bump-version.sh: version を持つ 3 ファイル（package.json / src-tauri/Cargo.toml /
# src-tauri/tauri.conf.json）を一括で書き換え、`cargo check` で src-tauri/Cargo.lock を
# 追従させる。
#
# 背景: 13 本のタグを実測で照合したところ、v0.5.0 / v0.5.1 / v0.5.7 の 3 本（23%）で
# 3 ファイルの値が食い違っていた（いずれも旧バージョンのままタグを打ってしまっていた）。
# 3 箇所を手で揃える運びには防止策が無く、人の注意力にしか依存していなかった。
# このスクリプトはその防止策であり、以下を守る:
#   - 食い違いを「直す」のではなく「検知して止める」（食い違いこそが防ぎたい状態なので、
#     黙って上書きしない）
#   - コミットやタグ付けはしない（人が行う。docs/design/plan.md の「リリース手順」参照）
#
# 使い方:
#   sh scripts/bump-version.sh <new-version>   # 例: sh scripts/bump-version.sh 0.5.9
#
# やること（この順で、途中で異常があれば何も書き換えずに止まる）:
#   1. 引数が SemVer (X.Y.Z、数字のみ) 形式か検証
#   2. 3 ファイルの現在値を読み、全部一致しているか確認
#   3. 作業ツリーが clean か確認（git status --porcelain が非空なら停止）
#   4. 3 ファイルを新しい version に書き換え
#   5. `cargo check` を走らせ、src-tauri/Cargo.lock を追従させる
#   6. 4 ファイルすべてを読み直し、新しい値になっていることを検証して表示

set -eu

usage() {
    echo "使い方: sh scripts/bump-version.sh <new-version>" >&2
    echo "例:     sh scripts/bump-version.sh 0.5.9" >&2
    exit 1
}

if [ "$#" -ne 1 ]; then
    usage
fi

NEW_VERSION="$1"

# (1) SemVer (X.Y.Z、数字のみ) 形式か検証。pre-release/build metadata は
# このプロジェクトの規約（plan.md §4「SemVer 0.y.z、1.0 まで」）に無いので受け付けない。
if ! echo "$NEW_VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "error: '$NEW_VERSION' は SemVer (X.Y.Z、数字のみ) 形式ではありません" >&2
    exit 1
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

PKG_JSON="$ROOT_DIR/package.json"
CARGO_TOML="$ROOT_DIR/src-tauri/Cargo.toml"
TAURI_CONF="$ROOT_DIR/src-tauri/tauri.conf.json"
CARGO_LOCK="$ROOT_DIR/src-tauri/Cargo.lock"

for f in "$PKG_JSON" "$CARGO_TOML" "$TAURI_CONF"; do
    if [ ! -f "$f" ]; then
        echo "error: $f が見つかりません" >&2
        exit 1
    fi
done

# --- 現在値を読む ------------------------------------------------------

read_json_version() {
    # 先頭の "version": "..." だけを対象にする（package.json / tauri.conf.json とも
    # トップレベルの version キーは 1 個だけの前提。複数あっても最初の 1 個しか見ない）。
    awk '
        !done && match($0, /"version": *"[^"]*"/) {
            v = substr($0, RSTART, RLENGTH)
            sub(/"version": *"/, "", v)
            sub(/"$/, "", v)
            print v
            done = 1
        }
    ' "$1"
}

read_cargo_toml_version() {
    # [package] section 内の最初の `version = "..."` だけを対象にする
    # ([dependencies] 等が独自の version フィールドを持つ可能性があるため)。
    awk '
        /^\[package\]/ { in_pkg = 1; next }
        /^\[/ { in_pkg = 0 }
        in_pkg && !done && match($0, /^version = "[^"]*"/) {
            v = substr($0, RSTART, RLENGTH)
            sub(/^version = "/, "", v)
            sub(/"$/, "", v)
            print v
            done = 1
        }
    ' "$1"
}

CUR_PKG=$(read_json_version "$PKG_JSON")
CUR_CARGO=$(read_cargo_toml_version "$CARGO_TOML")
CUR_TAURI=$(read_json_version "$TAURI_CONF")

if [ -z "$CUR_PKG" ] || [ -z "$CUR_CARGO" ] || [ -z "$CUR_TAURI" ]; then
    echo "error: version を読み取れないファイルがあります" >&2
    echo "  package.json           : ${CUR_PKG:-(読めず)}" >&2
    echo "  src-tauri/Cargo.toml    : ${CUR_CARGO:-(読めず)}" >&2
    echo "  src-tauri/tauri.conf.json: ${CUR_TAURI:-(読めず)}" >&2
    exit 1
fi

# (2) 3 ファイルの現在値が全部一致しているかを先に確認する。
# 食い違っていたらここで止める — これが本スクリプトの主目的そのもの。
if [ "$CUR_PKG" != "$CUR_CARGO" ] || [ "$CUR_PKG" != "$CUR_TAURI" ]; then
    echo "error: 3 ファイルの現在の version が食い違っています。黙って上書きしません。" >&2
    echo "  package.json             : $CUR_PKG" >&2
    echo "  src-tauri/Cargo.toml      : $CUR_CARGO" >&2
    echo "  src-tauri/tauri.conf.json : $CUR_TAURI" >&2
    echo "先に 3 ファイルを手で揃えてから、改めて実行してください。" >&2
    exit 1
fi

CUR_VERSION="$CUR_PKG"
echo "現在の version: $CUR_VERSION (3 ファイル一致)"

if [ "$CUR_VERSION" = "$NEW_VERSION" ]; then
    echo "error: 新しい version ($NEW_VERSION) が現在値と同じです" >&2
    exit 1
fi

# (3) 作業ツリーが clean かどうかを確認する。
if [ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]; then
    echo "error: 作業ツリーが clean ではありません。コミットまたは stash してから実行してください。" >&2
    git -C "$ROOT_DIR" status --short >&2
    exit 1
fi

echo "$CUR_VERSION -> $NEW_VERSION に書き換えます..."

# --- 書き換え -----------------------------------------------------------

write_json_version() {
    file="$1"
    new="$2"
    tmp="$file.bump-tmp"
    awk -v new="$new" '
        !done && match($0, /"version": *"[^"]*"/) {
            pre = substr($0, 1, RSTART - 1)
            post = substr($0, RSTART + RLENGTH)
            print pre "\"version\": \"" new "\"" post
            done = 1
            next
        }
        { print }
    ' "$file" > "$tmp"
    mv "$tmp" "$file"
}

write_cargo_toml_version() {
    file="$1"
    new="$2"
    tmp="$file.bump-tmp"
    awk -v new="$new" '
        /^\[package\]/ { in_pkg = 1; print; next }
        /^\[/ { in_pkg = 0 }
        in_pkg && !done && match($0, /^version = "[^"]*"/) {
            print "version = \"" new "\""
            done = 1
            next
        }
        { print }
    ' "$file" > "$tmp"
    mv "$tmp" "$file"
}

write_json_version "$PKG_JSON" "$NEW_VERSION"
write_cargo_toml_version "$CARGO_TOML" "$NEW_VERSION"
write_json_version "$TAURI_CONF" "$NEW_VERSION"

# (5) Cargo.lock を追従させる（cargo check は、対象パッケージ自身の version の
# 変更だけなら通常フルビルドを伴わず短時間で終わる。target/ が無い初回のみ遅い）。
echo "cargo check で Cargo.lock を追従させています..."
(cd "$ROOT_DIR/src-tauri" && cargo check --quiet)

# --- 検証 -----------------------------------------------------------

VERIFY_PKG=$(read_json_version "$PKG_JSON")
VERIFY_CARGO=$(read_cargo_toml_version "$CARGO_TOML")
VERIFY_TAURI=$(read_json_version "$TAURI_CONF")
VERIFY_LOCK=$(awk '
    /^name = "ptygrid"$/ { found = 1; next }
    found && match($0, /^version = "[^"]*"/) {
        v = substr($0, RSTART, RLENGTH)
        sub(/^version = "/, "", v)
        sub(/"$/, "", v)
        print v
        exit
    }
' "$CARGO_LOCK")

FAIL=0
for pair in "package.json:$VERIFY_PKG" "src-tauri/Cargo.toml:$VERIFY_CARGO" \
            "src-tauri/tauri.conf.json:$VERIFY_TAURI" "src-tauri/Cargo.lock:$VERIFY_LOCK"; do
    name="${pair%%:*}"
    val="${pair#*:}"
    if [ "$val" = "$NEW_VERSION" ]; then
        echo "  OK   $name -> $val"
    else
        echo "  NG   $name -> ${val:-(読めず)} (期待値 $NEW_VERSION)" >&2
        FAIL=1
    fi
done

if [ "$FAIL" -ne 0 ]; then
    echo "error: 一部のファイルが新しい version に揃っていません。手で確認してください。" >&2
    exit 1
fi

echo "完了: 4 ファイルとも $NEW_VERSION に揃いました。"
echo "コミットとタグ付けは人が行うこと（docs/design/plan.md の「リリース手順」参照）。"
