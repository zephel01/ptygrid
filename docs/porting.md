# クロスプラットフォーム移植ガイド (Linux / Windows)

現状は **macOSを主対象**とし、Linuxは**テスト対応（beta）**です。本ドキュメントは
Phase 3.9で整備したLinuxのbuild・配布手順と、今後のWindows対応に向けた作業の見取り図です。
コードは移植前提で書かれているため大改修は不要で、目安は次のとおりです。

| ターゲット | 規模感 | 主な作業 |
|---|---|---|
| **Linux** | **Phase 3.9テスト対応** | Ubuntu CI + PATH復元 + `.deb` / AppImage。実機互換性は検証継続中 |
| **Windows** | 中(1〜2週間) | `process_name` 実装 + PowerShell 差異の吸収 + テスト追加 + インストーラ |

> 注: Windows 経路は `#[cfg(windows)]` 分岐が実装済みでコンパイルは通る見込みですが、
> 自動テストで一度も踏まれていない(未検証)ことが実質的な工数の中心です。

---

## 既に対応済み(移植の土台)

一番の難所である PTY 層は `portable-pty 0.9`(wezterm 製)で、Windows=ConPTY /
Unix=forkpty を内部で吸収しています。加えて要所にプラットフォーム分岐が入っています。

| 箇所 | ファイル | 対応内容 |
|---|---|---|
| デフォルトシェル | `src-tauri/src/pty.rs` | Windows=`powershell.exe` / 他=`$SHELL`(fallback `/bin/bash`) |
| ホームディレクトリ | `src-tauri/src/pty.rs` | `USERPROFILE` / `HOME` |
| シェルラップ実行 | `src-tauri/src/session.rs` | `/bin/sh -c` / `powershell.exe -Command` |
| リサイズ | `src-tauri/src/session.rs` | `#[cfg(unix)]` ガードあり(非 unix パスも存在) |
| コンソール抑制 | `src-tauri/src/main.rs` | `windows_subsystem = "windows"` 属性 |
| プロセス名解決 | `src-tauri/src/pty.rs` | Linux=`/proc/<pid>/comm` / 他 unix=`ps` / **Windows=未実装(None)** |

- 依存(tokio / serde / notify / rmcp / axum)・フロント(Svelte 5 + xterm.js)は全て可搬。
- キーバインドを JS 側で横取りしていないため、Cmd/Ctrl の作り分けは不要。
- Tauri capability は `core:default` のみで OS 固有プラグイン依存なし。
- `scripts/queen-send.py` は Python なのでそのまま可搬。

---

## Linux テスト対応状況（Phase 3.9）

Linuxは`process_name`の`/proc`専用パスを含め、runtime経路を実装済みです。
platform固有の継続検証と配布はUbuntu CIへ固定しています。現段階では安定版サポートではなく、
Ubuntu / Debian系を中心に利用者の実機環境で検証を重ねるbeta扱いです。

- [x] Tauri v2 Linux build依存をUbuntu 22.04 CIへ導入
- [x] macOS / Ubuntu matrixでfrontend check/build、Rust test/clippy、Tauri native buildを実行
- [x] Linuxのforeground process名を`/proc/<pid>/comm`から取得
- [x] Queenを127.0.0.1だけにbindし、全PTYへ`QUEEN_URL`を注入
- [x] desktop launcherで欠落するuser shellの`PATH`を起動直後に復元
- [x] `bundle.active`を有効化し、Linux targetを`.deb` / AppImageへ固定
- [x] tag / manual workflowでLinux packageを生成してartifact化
- [x] README / user guideへLinux導入・build手順を追加

### 対応基準とbuild

glibc互換性の下限を不用意に上げないため、Linux bundleはUbuntu 22.04 runnerを基準にします。
Debian 12もWebKitGTK 4.1を標準提供する互換baselineです。

```bash
npm run tauri dev
npm run bundle:linux
```

GitHub Actionsの通常CIはmacOS / Linux両方でnative applicationを`--no-bundle` buildします。
`.deb` / AppImage生成はtag pushまたは`workflow_dispatch`で実行します。

## Windows 対応チェックリスト

分岐のガワはあるが、機能・品質面で埋める箇所がある。

- [ ] **`process_name()` の Windows 実装**(最優先)
      現状 `None` を返すため、Queen の `read_output` / `send_message` / `spawn_agent` を
      「フォアグラウンドのプロセス名(例: `codex`)」で指定する経路が機能せず、`#<id>` と
      定義名しか使えない。`SessionInfo.foreground` も空になる。
      Windows API(`QueryFullProcessImageNameW`)または `sysinfo` クレートで実装する。
- [ ] **PowerShell の差異吸収**
      `shell_wrap` が `powershell.exe -Command` を使う。`/bin/sh -c` とクォート/パイプ/
      変数展開の意味が異なるため、sh 前提の `cmd` 記法は動かない。
      `mterm.yml` の `cmd` に関する注意書きをドキュメント化する、または `cmd.exe` /
      `pwsh` を選べるようにするか検討。
- [ ] **テストの Windows 対応**
      既存テストは `/bin/cat`・`/bin/sh` に依存し Unix 専用。Windows 用の等価コマンド
      (例: `cmd /c type`, `more`)に切り替えるか `#[cfg]` で分岐し、Windows CI で緑にする。
- [ ] ConPTY の VT ストリームに対する `ansi.rs` の CR(`\r`)畳み込み挙動を実機確認
- [ ] パス絶対判定(`Path::is_absolute` はクロス対応だが `resolve_cwd` を実機確認)
- [ ] アイコン: 現状 `src-tauri/icons/icon.png` のみ。Windows 用 `.ico` を用意(Tauri で生成可)
- [ ] インストーラ: MSI / NSIS、コード署名
- [ ] README のプラットフォームバッジに Windows を追加

---

## 共通で必要な整備

- [ ] GitHub Actions を3-OSへ拡張（macOS / ubuntuは対応済み、windowsを追加）
- [x] `tauri.conf.json` の `bundle.active` を有効化し、Linux targetを設定
- [ ] リリース時のバイナリ配布(各 OS のインストーラ)

## 検証の観点(移植後に必ず踏む)

- PTY: 生成 / 入出力 / resize / kill / autorestart(never/on-failure/always, 連続5回打ち切り)
- Queen: `list_agents` / `read_output`(cursor/erase/alternate screen再構成) / `send_message` /
  `spawn_agent`(許可リスト) / `notify`、bind が 127.0.0.1 のみか
- config: `mterm.yml` 読み込み、`${VAR}` 展開、autostart、変更監視での Reload
- UI: 最大9ペイン、レイアウト、状態ドット(running / exited / restarting + exit code)

## 参照

- [Tauri v2 Linux prerequisites](https://v2.tauri.app/start/prerequisites/#linux)
- [Tauri v2 AppImage guide](https://v2.tauri.app/distribute/appimage/)
- [Tauri v2 Debian package guide](https://v2.tauri.app/distribute/debian/)
- [tauri-apps/fix-path-env-rs](https://github.com/tauri-apps/fix-path-env-rs)
