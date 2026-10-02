# 2026-10-02 OSCQuery / OAuth / ロゴ検証

- `cargo test --workspace`: 42件成功。
- `cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all --check`: 成功。
- Pulsoidの本番device_authorization endpointへ指定client_idとscopeをPOSTし、device_code発行を確認。expires_in=900、interval=3。device_code/user_codeをログに出さず、ユーザー承認や実トークン取得は行っていない。
- OAuthのHTTP模擬サーバーテストでclient_id/scope/device grant、authorization_pendingから成功、拒否、期限切れを検証。slow_down等の分類も検証。
- `python3 tests/oscquery_smoke.py target/debug/nemuribeat`: 模擬mDNS広告とOSCQuery HTTPサーバーから宛先を発見し、手動設定の宛先を使わずOSCを受信。HTTP 503時の送信停止、復帰時の再送信、計測/DB保存継続、正常終了を確認。
- この試験で依存ライブラリのHTTP取得がstatusを検証しない点を検出し、OSCQuery生存確認はstatus確認・4MiB上限・2秒timeoutを持つHTTP処理へ変更した。
- 96秒の既存統合試験: HR＋RR Raw96件/Session1件、HRのみRaw97件/Session0件。API/IPC/手動OSC/DBロック中の計測継続/flush成功。
- 指定PNG/ICOを同梱。eguiアイコンとサイドバーに使用。macOS ICNSはPNG各サイズを格納し、OS画像デコーダーで読み出して図案を確認。バンドル生成・Info.plist・ad-hoc署名検証成功。
- Windows EXEアイコン埋め込みをbuild.rsに追加。Windowsビルド/実行、実VRChatアバターへの適用、ブラウザ承認後の実Pulsoid心拍受信、更新後のGUI全画面目視は未検証。

以下は変更前の検証記録。

---

# 2026-10-02 NemuriBeat 検証追記

- `cargo test --workspace`: 37件成功。
- `cargo clippy --workspace --all-targets -- -D warnings`: 成功。
- `cargo fmt --all --check`: 成功。
- macOS `target/NemuriBeat.app` のビルド・署名・実ウィンドウ起動確認。
- 実画面でNemuriBeatのタイトル、新しいSettings、Sleepのグレーアウトを確認。
- UI描画テストでHRのみのRR/HRV部品除去、RRありでの部品維持を確認。
- 96秒の統合テスト（HR＋RR）: Raw96件・自動Session1件・終了flush成功。
- 96秒の統合テスト（HRのみ）: Raw96件・睡眠Session0件。擬似睡眠Asleep=trueをAPIで確認。
- 両モードでAPI / IPC / ローカルOSC受信 / DBロック中の計測継続 / ソケット後片付けを確認。
- 最小継続時間、モード切替時のリセット、RR喪失時の記録停止、固定サイズ集計を単体テスト。
- 旧schema/WALの移行、元DBとpendingファイルの保持、Cache保持期限処理を単体テスト。
- 実Pulsoidトークンは使用していない。実サービス認証・配信の受信は未検証。
- Windowsのパス分岐は実装・パステスト済み、Windows上のアプリ実行は未検証。
- 実際の睡眠判定精度や一晩の実センサー運用は未検証。

---

# 検証記録（2026-10-01）

環境: macOS / Apple Silicon、Rust 1.98.1。依存はCargo.lockに固定。

| 検証 | 結果 |
|---|---|
| cargo fmt --all | 適用済み |
| cargo clippy --workspace --all-targets -- -D warnings | 成功、警告なし |
| cargo test --workspace | 18件成功 |
| cargo build -p physio-desktop | 成功 |
| macOS .app生成・Info.plist・ad-hoc署名 | 成功 |
| native GUI | 起動・Sensors画面・BLEスキャン結果の表示を確認 |
| 75秒デモ | Raw 75件、1分区間2件、終了済みSession、dropped=0 |
| tests/smoke.py（65秒） | Raw 66件、1分区間2件、dropped=0、正常終了 |
| REST | latest / settings / sessions / session detail / 不明IDの404を確認 |
| Unix IPC | JSON Snapshot取得・終了時ソケット削除を確認 |
| OSC | ローカルUDP受信・OSCエンコードと型・正規化を確認 |
| DB書込ロック | ロック中にもsequence_idが増加することを統合テストで確認 |

タイマーの初回tickと終了タイミングにより、65秒のデモで通知が66件になる場合があります。通知sequenceは連続し、保存数と一致しています。

## ユニットテストの対象

- HRの8/16 bit、複数RR、Energyフィールド、Contact、切れたBLE payload
- Ring上限、stale検知、切断時Tracking解除
- 既知RR列のRMSSD / SDNN / pNN50、Artifactを跨がない差分、RR停止
- Baseline不足・接続断、120秒の睡眠移行条件、欠測を睡眠時間に加算しない処理
- SQLiteのRaw全RR保持、冪等再投入、終了flush、DBロック時のqueue overflow記録、pending spool再投入
- OSCの型・ローカルUDP・HeartOSCの正規化/桁互換

## 制限

- シェルからのGUI起動はLaunchServicesエラー/終了コード134となりましたが、ネイティブアプリ操作経由では実際のウィンドウ表示を確認できました。シェル側の制約原因は確定していません。
- デバイス一覧の日本語名にフォント不足が見えたため、最後にOSのCJKフォントを読み込むfallbackを追加しました。この変更はビルド・Clippy確認済みですが、再起動後の日本語描画は未確認です。
- GUIの全画面・操作の網羅的QA、Coospo実機接続、実睡眠の正解ラベル評価、VRChat接続は未実施です。
- Windows / Linux用CI定義は追加しましたが、この環境では実行していません。
- GUI/終了処理の最終調整後にも再ビルドし、短時間headless終了を確認しています。
- OSCQuery / Shared Memory / Windows Named Pipe / ONNXは未実装です。

設計書のv1完成条件を満たしたと判定するには、docs/IMPLEMENTATION.mdの受入試験と残件対応が必要です。
