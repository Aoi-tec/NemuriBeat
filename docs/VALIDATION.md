# OyasumiVR追加検証（2026-10-03）

- `cargo test --workspace`: 57件成功、既存の専用Python環境が必要なGRU試験1件は通常実行ではignored（前回の実重み検証結果は下記）。
- OyasumiVR連携の実ワーカーをローカルHTTP/UDP相手に接続し、覚醒→睡眠→判定不能→無効化→復帰→終了を確認。判定不能・無効・終了では覚醒命令を送らない。
- `python3 tests/oyasumi_smoke.py target/debug/nemuribeat`: 26秒のHRのみデモで、mDNS自動検出、HOST_INFO検証、SleepMode Bool読戻し、状態一致時の再送抑制、HTTP断中の保持、動的UDPポート変更後の復帰、VRChat OSC OFF時の独立動作、Raw保存継続を確認。
- `cargo clippy --workspace --all-targets -- -D warnings`、fmt、diff check成功。
- 実OyasumiVR、Windows、SteamVR、自動操作の適用は未検証。`docs/OYASUMIVR.md` と `Task.MD` に手順・残件を記載。

---

# 2026-10-03 SleepECG公式GRU / OpenVR / 日次集約

- `cargo test --workspace`: 54件成功、専用環境が必要な公式モデル試験1件は通常実行ではignore。別途その試験を有効にし、実GRUのPythonワーカー通信・入力世代破棄・終了を確認して成功。
- `cargo clippy --workspace --all-targets -- -D warnings`: 成功。
- Python入力試験5件成功。実RRの保持、HRのみから拍動を作らないこと、欠測・接触不良のリセット、近接情報のtrue/false/unknownを確認。
- Python 3.12 / sleepecg 0.5.9 / TensorFlow 2.16.2 / Keras 3.15.1で、公式ws-gru-mesaの36特徴量・3出力とlookback240/lookforward270を実物から確認。人工RR列の実重み推論は有限確率を返し、対象epochの遅延とstdoutのJSON通信を確認。これは判定精度の試験ではない。
- HMDロジックの試験: 厳密0の平均を棄却、交互加速度の相殺を防止、未装着・追跡断・サンプリング断の棄却、有界30秒窓。
- 日次集約の試験: 日跨ぎ・再接続区間を同じ日へまとめ、空白を睡眠時間へ加えない。GUIのグラフに全区間を含み、日単位の記録中表示を確認。
- `sleep_runtime_smoke.py`: isolated DBの2区間60分＋90分を1日150分へ集約、日別グラフ、元の区間APIの互換、未知日の404、RRなしTracking=false、Python未設定でも基本機能が動作し終了できることを実プロセスで確認。GRUはRR入力まで遅延ロードする。Python実行ファイルがない場合の実際の推論開始エラーはRust試験で確認。
- 96秒の既存回帰試験: HR＋RR Raw96件/Session1件、HRのみRaw96件/Session0件。API/IPC/OSC/DBロックの分離/flush成功。
- OSCQueryのmDNS/HTTP/UDP・停止・復帰の回帰成功。Web背景ラインのNode試験成功。macOSバンドルを再生成。
- OpenVR実機、睡眠の外部正解ラベルによる精度、WindowsのTensorFlow/OpenVR環境、Native GUIの新しいSettings操作、XSOverlay/OVR Toolkit内の表示は未検証。残件はルートTask.MD。

以下は以前の変更の検証記録。

# 2026-10-03 常駐 / Built-in Web Browser / HypeRate検証

- 背景の心電図風ライン追加後: `node tests/overlay-web.js` で75 BPMの6秒間7〜8拍、120 BPMの12拍、停止時の水平線、600点上限、非表示/動きを減らす設定での停止、復帰を確認。`node --check`・diffチェック成功。実ブラウザの通常/コンパクト＋透過表示でライン描画を確認。デスクトップを再ビルドしmacOSバンドルを再生成。
- Rustの45件成功、Clippy警告なし、整形チェック成功。
- sensor-pulsoidをsensor-hyperateへ置換。WebSocket模擬サーバーでチャンネル参加、他ID/システムフレームの除外、新着HR（同じBPMを含む）、中止・離脱を確認。公開APIにないRR/測定元timestampを生成しないことを検証。
- APIルーター試験でWebのみ有効時に設定APIを公開しないこと、キャッシュ無効化、受信停止/切断時のHR null、無効時のページ404を確認。
- Node.jsで実際の同梱Webスクリプトを実行し、心拍・モード表示、データ停止/通信エラー時の値消去、復帰、直列ポーリングを確認。
- ブラウザ実表示でNemuriBeatロゴ、DEMO、72 BPM、HR ONLY、擬似判定、更新秒数の描画・更新を確認。
- 常駐の閉じる分岐を単体検証。トレイが作成できた場合のみ画面を隠し、未対応/失敗時と明示終了では通常終了する。トレイ作成はmain event loop開始後に行う。
- 96秒の既存統合試験: HR＋RR Raw96件 / Session1件、HRのみRaw96件 / Session0件。API/IPC/手動OSC/DB-lock isolation/flush成功。
- OSCQuery模擬mDNS/HTTP/UDPの自動検出、停止、復帰、DB保存継続を再検証。
- macOSバンドルのビルド、署名、Info.plist確認。
- 最新のユーザー指示に従い、OVR ToolkitもXSOverlayと共通HTTP/HTML方式とした。C# DLLは今回の出力に含めない。
- Windows実行・トレイ操作の実機確認、XSOverlay/OVR Toolkit内での表示、実HypeRate APIキーを使った受信は未検証。

以下は変更前の検証記録。

---

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
