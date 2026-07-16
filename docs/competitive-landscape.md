# 競合調査: 類似ツールの比較 (Competitive Landscape)

調査日: 2026-07-16(grok による Web 横断調査、32+ サイト。docs/design.md の記述を起点に更新)

> 外部projectの数値は調査日時点のsnapshot。ptygridの実装状況はPhase 3.7へ更新済み。

## design.md (2026-07-15) からのアップデート

| design.md の記述 | 今回の調査結果 |
|---|---|
| cmux ~17k stars | ~24.5k に成長。macOS ネイティブ端末の覇権候補 |
| Superset ~10.7k・「ターミナル」 | ~12.4k。実態はエージェント IDE(Electron, ELv2) |
| Crystal 要確認 | MIT。Nimbalyst に移行(Crystal は legacy 扱い) |
| Parallel Code ~633 | ~850、MIT、Electron+Solid、worktree 系の定番 |
| 類似は少数 | Architect(Zig グリッド+MCP)、Agent Deck、Conductor が追加で重要 |
| fork せず自作 | 依然妥当。近い OSS は GPL/AGPL/ELv2 か、worktree 思想が違う |

## ポジショニング(結論)

市場は大きく2系統に分岐している:

- **worktree で隔離する系**: Claude Squad / Parallel Code / Conductor / Superset
  — エージェントごとに git worktree を切って並列実行、diff レビューで統合
- **同一画面で協調する系**: HiveTerm / **ptygrid**
  — マルチペイン + オーケストレーター(Queen)でエージェント同士が読み書きし合う

```
                協調・MCP・config-as-code が強い
                          ↑
              HiveTerm ●  │  ● ptygrid  ← ここを埋めている
                          │
    Architect ●           │            ● cmux
  ← worktree 並列が強い ──┼── 端末プリミティブが強い →
 Claude Squad ●           │
Parallel Code ●           │
    Conductor ●           │
     Superset ●           ↓ (IDE化・機能最大)
                オーケストレーションは薄い / 別モデル
```

**空白地帯**: HiveTerm 相当の「Tauri + YAML 設定 + 内蔵 MCP」は OSS にほぼ存在しない。
cmux は端末として最強だが Queen 型ではない。Superset / Conductor は worktree IDE。

## 参考にする価値が高いコードベース

| 優先 | リポジトリ | 学ぶ点 |
|---|---|---|
| 高 | Architect | グリッド端末、status highlight、薄い MCP |
| 高 | Claude Squad | セッション管理・worktree(**AGPL のため設計参考のみ**) |
| 中 | Parallel Code | worktree UI、diff review(Phase 3 の参考) |
| 中 | cmux | 通知 UX、CLI プログラマビリティ(GPL・macOS 専用) |
| 低(仕様のみ) | HiveTerm | 製品 UX・Queen tools の上限仕様 |

## ptygrid の勝ち筋 / 次に取る機能

**すでに持っている強み(Phase 3.7)**: マルチペイン + mterm.yml + Queen 17 toolsに加え、
Git review/commit、任意worktree分離、logical resume、process-tree resource監視、競合安全な
Pins/Notes、永続Inbox/Replyまで一体化している。

| 優先 | 内容 | 競合が強い領域 |
|---|---|---|
| 実装済み | Git diff/commit、worktree option、pins/notes、inbox/reply | Superset / Parallel Code / Conductor / HiveTerm |
| 次: Phase 3.8 | cancellable await | HiveTerm 本体 |
| UX | 通知リング / 「要承認」ハイライト | cmux / Architect |
| 差別化維持 | config-as-code + 許可リスト付き spawn | ほとんど誰も両立していない |

## やらない方がいいこと

- cmux との「フル端末エミュレータ」競争(Ghostty ネイティブに負ける)
- Superset の「フル IDE + クラウド remote」競争(スコープ爆発)
- Claude Squad のコード fork(AGPL)

## 一言まとめ

- **製品としての最接近**: HiveTerm(クローズド・仕様の本家)
- **OSS スター最大**: cmux(端末プリミティブ)と Superset(worktree IDE)
- **思想の分岐**: worktreeで隔離する系 vs 同一画面で協調する系 → ptygridは後者を基本にしつつopt-in isolationも提供
- **自作の正当性**: 近い OSS はライセンスか設計思想がズレており、「Tauri + mterm.yml + Queen」の OSS 実装は依然ほぼ空白
