#!/bin/sh
# bump-version.sh: version を持つ 3 ファイル（package.json / src-tauri/Cargo.toml /
# src-tauri/tauri.conf.json）を一括で書き換え、2 つの lock ファイル
# （package-lock.json / src-tauri/Cargo.lock）を追従させる。
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
#   5. `npm install --package-lock-only` で package-lock.json を追従させる
#   6. `cargo check` を走らせ、src-tauri/Cargo.lock を追従させる
#   7. 5 ファイルすべてを読み直し、新しい値になっていることを検証して表示
#
# 4 以降で異常が起きた場合（典型は 6 の `cargo check` 失敗）は、書き換えた 5 ファイルを
# 退避しておいた原本から戻してから止まる。戻さないと「3 ファイルは新版・lock は旧版」
# という、このスクリプトが防ぐために存在する食い違いがそのまま残り、しかも次の実行は
# 3 の dirty-tree ガードに弾かれて手で戻すしかなくなる。
#
# 5 について: `npm install --package-lock-only` は version フィールドを揃えるだけでなく、
# semver range 内での依存の**再解決**も行う（実測: postcss 8.5.19 -> 8.5.26 のような
# patch 更新が同時に入る）。version bump のコミットに依存更新が黙って混ざるのは、
# 3 ファイルの食い違いと同じ種類の事故なので、このスクリプトは差分を検査し、
# version 以外の行が動いていたら**書き換えを取り消して止まる**。その場合は先に
# `npm install` を単独で回し、依存更新だけを別コミットにしてから改めて bump する。
#
# package-lock.json が version を持つのは 2 箇所（トップレベルと packages[""]）で、
# 両方を検証する。npm が無い環境では、黙って読み飛ばすと防ぎたい食い違いを作るだけなので
# エラーで止める（package-lock.json 自体が無いリポジトリでは対象外として飛ばす）。

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
PKG_LOCK="$ROOT_DIR/package-lock.json"

for f in "$PKG_JSON" "$CARGO_TOML" "$TAURI_CONF"; do
    if [ ! -f "$f" ]; then
        echo "error: $f が見つかりません" >&2
        exit 1
    fi
done

# package-lock.json は「あるなら必ず揃える」対象。無いリポジトリでは対象外として飛ばすが、
# あるのに npm が無い環境で黙って飛ばすと、このスクリプトが防ぐための食い違いを自分で
# 作ることになるので、そこは止める。
if [ -f "$PKG_LOCK" ]; then
    HAVE_PKG_LOCK=1
    if ! command -v npm >/dev/null 2>&1 || ! command -v node >/dev/null 2>&1; then
        echo "error: package-lock.json がありますが npm / node が見つかりません。" >&2
        echo "  Node.js を用意するか、package-lock.json を手で $NEW_VERSION に揃えてください。" >&2
        exit 1
    fi
else
    HAVE_PKG_LOCK=0
fi

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

read_lock_self_version() {
    # package-lock.json の packages[""] ブロック（= このパッケージ自身のエントリ）が持つ
    # version。トップレベルの "version" は read_json_version が拾うので、こちらは
    # 2 つ目の在処を独立に読むためのもの。npm の整形に依存しすぎないよう空白は緩く見る。
    awk '
        /"packages"[[:space:]]*:[[:space:]]*\{/ { in_packages = 1; next }
        in_packages && /^[[:space:]]*""[[:space:]]*:[[:space:]]*\{/ { in_self = 1; next }
        in_self && match($0, /"version"[[:space:]]*:[[:space:]]*"[^"]*"/) {
            v = substr($0, RSTART, RLENGTH)
            sub(/"version"[[:space:]]*:[[:space:]]*"/, "", v)
            sub(/"$/, "", v)
            print v
            exit
        }
        in_self && /^[[:space:]]*\},?[[:space:]]*$/ { exit }
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

# 最初の 1 バイトを書く前に原本を退避する。ここから先の異常終了は、成功していない
# 書き換えを残したまま抜けることになるので、EXIT trap で必ず元に戻す。Cargo.lock も
# 対象に含める — (5) が通ったあとに (6) で落ちた場合、3 ファイルだけ戻すと今度は
# Cargo.lock だけが新版になり、食い違いの向きが変わるだけになる。
BACKUP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/ptygrid-bump.XXXXXX")
cp "$PKG_JSON" "$BACKUP_DIR/package.json"
cp "$CARGO_TOML" "$BACKUP_DIR/Cargo.toml"
cp "$TAURI_CONF" "$BACKUP_DIR/tauri.conf.json"
if [ -f "$CARGO_LOCK" ]; then
    cp "$CARGO_LOCK" "$BACKUP_DIR/Cargo.lock"
fi
if [ "$HAVE_PKG_LOCK" -eq 1 ]; then
    cp "$PKG_LOCK" "$BACKUP_DIR/package-lock.json"
fi

restore_backup() {
    cp "$BACKUP_DIR/package.json" "$PKG_JSON"
    cp "$BACKUP_DIR/Cargo.toml" "$CARGO_TOML"
    cp "$BACKUP_DIR/tauri.conf.json" "$TAURI_CONF"
    if [ -f "$BACKUP_DIR/Cargo.lock" ]; then
        cp "$BACKUP_DIR/Cargo.lock" "$CARGO_LOCK"
    fi
    if [ -f "$BACKUP_DIR/package-lock.json" ]; then
        cp "$BACKUP_DIR/package-lock.json" "$PKG_LOCK"
    fi
}

on_exit() {
    status=$?
    if [ "$status" -ne 0 ]; then
        restore_backup
        echo "  → 書き換えを取り消し、$CUR_VERSION の状態に戻しました。" >&2
    fi
    rm -f "$PKG_JSON.bump-tmp" "$CARGO_TOML.bump-tmp" "$TAURI_CONF.bump-tmp"
    rm -rf "$BACKUP_DIR"
    exit "$status"
}
trap on_exit EXIT
# シグナルで死ぬときも EXIT trap を通す（POSIX sh は signal 単体では EXIT を走らせない）。
trap 'exit 1' HUP INT TERM

write_json_version "$PKG_JSON" "$NEW_VERSION"
write_cargo_toml_version "$CARGO_TOML" "$NEW_VERSION"
write_json_version "$TAURI_CONF" "$NEW_VERSION"

# (5) package-lock.json を追従させる。--package-lock-only なので node_modules には
# 触らないが、依存の再解決は行われる（後段のガード参照）。
if [ "$HAVE_PKG_LOCK" -eq 1 ]; then
    # lock は派生ファイルなので (2) の「3 ファイル一致」ガードの対象外。ただし黙って
    # 直すと、いつからズレていたのかが誰にも見えないままになるので、事実だけ報告する。
    PREV_LOCK_VERSION=$(read_lock_self_version "$BACKUP_DIR/package-lock.json")
    if [ "$PREV_LOCK_VERSION" != "$CUR_VERSION" ]; then
        echo "注意: package-lock.json は ${PREV_LOCK_VERSION:-(読めず)} で 3 ファイル ($CUR_VERSION) と食い違っていました。追従させます。"
    fi
    echo "npm install --package-lock-only で package-lock.json を追従させています..."
    if ! (cd "$ROOT_DIR" && npm install --package-lock-only --no-audit --no-fund); then
        echo "error: npm install --package-lock-only が失敗しました。" >&2
        exit 1
    fi

    # このパッケージ自身の version 以外が動いていないことを確認する。動いていたら
    # 依存の再解決が混ざったということなので、release の bump には持ち込まない。
    #
    # テキスト diff ではなく JSON として比較する。lock が（ドリフトで）3 ファイルとは
    # 別の値だった場合、旧値は CUR_VERSION とも NEW_VERSION とも違うので、行の中身から
    # 「自分の version 行かどうか」を見分けられない。構造で除外すれば位置で判別できる。
    UNEXPECTED=$(node -e '
        const fs = require("fs");
        const load = (p) => JSON.parse(fs.readFileSync(p, "utf8"));
        const [before, after] = [load(process.argv[1]), load(process.argv[2])];
        // 比較対象から外すのは「このパッケージ自身の version」2 箇所だけ。
        const strip = (o) => {
            const c = JSON.parse(JSON.stringify(o));
            delete c.version;
            if (c.packages && c.packages[""]) delete c.packages[""].version;
            return c;
        };
        const [a, b] = [strip(before), strip(after)];
        const out = [];
        for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) {
            if (key === "packages") continue;
            if (JSON.stringify(a[key]) !== JSON.stringify(b[key])) out.push(`(top-level) ${key}`);
        }
        const [pa, pb] = [a.packages || {}, b.packages || {}];
        for (const key of new Set([...Object.keys(pa), ...Object.keys(pb)])) {
            if (JSON.stringify(pa[key]) !== JSON.stringify(pb[key])) out.push(key || "(self)");
        }
        process.stdout.write(out.join("\n"));
    ' "$BACKUP_DIR/package-lock.json" "$PKG_LOCK")
    if [ -n "$UNEXPECTED" ]; then
        echo "error: package-lock.json に version 以外の差分が出ました（依存の再解決）。" >&2
        echo "$UNEXPECTED" | head -20 | sed 's/^/    /' >&2
        echo "  → 依存更新を release の bump に混ぜないでください。先に" >&2
        echo "     'npm install' を単独で回して別コミットにしてから、改めて bump してください。" >&2
        exit 1
    fi
fi

# (6) Cargo.lock を追従させる（cargo check は、対象パッケージ自身の version の
# 変更だけなら通常フルビルドを伴わず短時間で終わる。target/ が無い初回のみ遅い）。
# 明示的に受けるのは、`set -e` に任せると理由が 1 行も出ないまま抜けるため。
echo "cargo check で Cargo.lock を追従させています..."
if ! (cd "$ROOT_DIR/src-tauri" && cargo check --quiet); then
    echo "error: cargo check が失敗しました（version の書き換えとは無関係のビルドエラーの可能性があります）。" >&2
    exit 1
fi

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

# 関数はサブシェルではないので FAIL への代入はそのまま呼び出し側に残る。ループから
# 関数に変えたのは、package-lock.json の行だけが「ファイルが無ければ出さない」ため。
check_version() {
    if [ "$2" = "$NEW_VERSION" ]; then
        echo "  OK   $1 -> $2"
    else
        echo "  NG   $1 -> ${2:-(読めず)} (期待値 $NEW_VERSION)" >&2
        FAIL=1
    fi
}

check_version "package.json" "$VERIFY_PKG"
check_version "src-tauri/Cargo.toml" "$VERIFY_CARGO"
check_version "src-tauri/tauri.conf.json" "$VERIFY_TAURI"
check_version "src-tauri/Cargo.lock" "$VERIFY_LOCK"

if [ "$HAVE_PKG_LOCK" -eq 1 ]; then
    # version は 2 箇所（トップレベルと packages[""]）にあり、両方揃って初めて OK。
    # 片方だけ動いた状態を「OK」と表示しないよう、食い違いはそのまま値として見せる。
    VERIFY_PKG_LOCK_ROOT=$(read_json_version "$PKG_LOCK")
    VERIFY_PKG_LOCK_SELF=$(read_lock_self_version "$PKG_LOCK")
    if [ "$VERIFY_PKG_LOCK_ROOT" = "$VERIFY_PKG_LOCK_SELF" ]; then
        check_version "package-lock.json" "$VERIFY_PKG_LOCK_ROOT"
    else
        check_version "package-lock.json" \
            "root=${VERIFY_PKG_LOCK_ROOT:-?}/packages[\"\"]=${VERIFY_PKG_LOCK_SELF:-?}"
    fi
fi

if [ "$FAIL" -ne 0 ]; then
    echo "error: 一部のファイルが新しい version に揃っていません。手で確認してください。" >&2
    exit 1
fi

if [ "$HAVE_PKG_LOCK" -eq 1 ]; then
    echo "完了: 5 ファイルとも $NEW_VERSION に揃いました。"
else
    echo "完了: 4 ファイルとも $NEW_VERSION に揃いました（package-lock.json は対象外）。"
fi
echo "コミットとタグ付けは人が行うこと（docs/design/plan.md の「リリース手順」参照）。"
