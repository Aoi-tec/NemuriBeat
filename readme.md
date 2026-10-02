# NemuriBeat

BLE / Pulsoidの心拍を取得し、リアルタイム表示・睡眠状態出力・RRを用いた睡眠記録を行うRust製デスクトップアプリです。
既存のデザイン（テーマ、フォント、チャート）を維持しています。設計原本は `Architecture.md`、追加仕様は `docs/NEMURIBEAT.md` を参照してください。

## 起動

```sh
cargo run -p nemuribeat
cargo run -p nemuribeat -- --demo
cargo run -p nemuribeat -- --demo-hr-only
```

Rust 1.98.1 / Cargoが必要です。macOSはXcode Command Line Tools、WindowsはMSVC、LinuxはBlueZ/D-BusとGUI開発ライブラリが必要です。

macOSアプリバンドル:

```sh
./scripts/bundle-macos.sh --debug
open target/NemuriBeat.app
```

Windows:

```powershell
cargo build --release -p nemuribeat
.\target\release\nemuribeat.exe
```

## 入力と睡眠モード

| 入力 | Realtime | Sleep | 睡眠セッション | 外部出力 |
|---|---|---|---|---|
| HR＋RR | HR / RR / HRV | 有効 | 条件成立後に自動記録 | RRベースの状態 |
| Pulsoid、RRなしBLE | HR / 擬似睡眠 | グレーアウト | 作成しない | 擬似睡眠の状態 |
| 切断、HR無受信 | 判定不能 | 無効 | 記録終了 | Tracking=false、Asleep=false |

- Sensors → BLEスキャン・接続、または「PulsoidでOAuth認証」→ 表示リンクをブラウザで開いて承認。
- Pulsoidは `data:heart_rate:read` 権限のアクセストークンを使用。トークンはメモリだけに保持し、DB・ログ・RESTに保存/公開しません。
- 入力の切替は先に「入力を切断 / 接続を中止」。再接続も停止できます。
- RR対応機器でもRRが5秒以上届かなくなるとHRのみへ切替。HRも5秒以上無受信なら判定不能。
- 睡眠判定は未校正のルールです。HRのみは擬似状態であり、睡眠計測として保存しません。

### Pulsoid

[公式Device Authorization Flow](https://docs.pulsoid.net/access-token-management/oauth-2-device-authorization-flow) を使用します。client_idは `5131ba31-97fa-4587-b9d8-e155640f343e`、scopeは `data:heart_rate:read`。Sensorsで認証を開始し、表示リンクを開いてブラウザで承認すると自動接続します。期限切れ・拒否時は再認証。入力切断で認証待ちも中止します。client_secretは使用せず、アクセストークンはメモリ内だけで保持します。headlessのみ既存トークンの環境変数入力を維持します。
[公式WebSocket](https://docs.pulsoid.net/read-heart-rate/read-heart-rate-via-websocket) の新着HRを即時反映し、切断時は1〜30秒のバックオフで再接続します。HTTPの高頻度ポーリングは行いません。供給される測定値以上の時間分解能を生成しません。

`timestamp`は受信側のUnixミリ秒、`source_timestamp`はPulsoidの`measured_at`です。送信元時計のずれで鮮度判定を誤らないよう分離しています。

```sh
# PULSOID_ACCESS_TOKENをプロセス環境に設定した場合のheadless起動
cargo run -p nemuribeat -- --headless --pulsoid --api
```

### 自動睡眠記録

RR睡眠推定は有効RR 30個・品質0.8以上・過去Baseline 30分以上が必要です。
Probabilityが0.75以上で設定時間（初期値5分、60〜1800秒）続くと睡眠と判定し、その時点でSessionを作成・保存します。継続判定の開始時刻を入眠候補時刻として、直前の集計も記録へ取り込みます。

覚醒側はProbabilityが0.40以下で2分継続。RR喪失・切断・判定不能では記録を終了します。手動終了後は一度覚醒側へ戻るまで再記録しません。Settingsで自動記録を無効にすることもできます。

HRのみは基準心拍（初期値60 bpm）と傾きを使う暫定ルール＋EMAで擬似睡眠を生成します。継続時間の条件は同じです。単なる低心拍を睡眠と判定する可能性があるため、実睡眠記録と混ぜません。

## 保存先

既定はWindowsの `%LOCALAPPDATA%\NemuriBeat`（AppData内）。OSのデータフォルダが取得/利用できなければ、実行ファイル横の `NemuriBeatData` を使います。
macOSは `~/Library/Application Support/NemuriBeat`。`--data-dir PATH`で明示できます。

```text
NemuriBeat/
  PersonalData/nemuribeat.sqlite  # 実測HR/RR、確定Session、設定、欠損情報。自動削除しない
  Cache/derived.sqlite           # 再生成可能な1分集計。35日保持
  Demo/                          # デモ専用。実データと分離
    PersonalData/nemuribeat.sqlite
    Cache/derived.sqlite
```

旧PhysioCoreの既定保存先、または`--data-dir`直下の旧`physio.sqlite`/`demo.sqlite`から、初回のみSQLite Backup APIで移行します。元DBとpendingファイルは残し、新しいデータを上書きしません。
Cache削除後もRaw・確定済みの睡眠記録は残ります。Baselineは残っているセッション特徴量か、新しく集まる有効特徴量から構成するため、Cache削除後に再度計測待ちとなる場合があります。実行中のDBファイルは削除しないでください。

メモリには20分/最大6000通知のRRバッファ、約30分/最大1805点の表示キャッシュ、最大36件の自動記録前集計を保持。履歴は選択した1セッションのみ（最大20160区間）読込みます。セッション中央値は固定長の1ms単位ヒストグラムで集計します。時間に比例してメモリを増やさず、長期のRawはDBへ保存します。

## OSC / 外部読み込み

OSCは初期状態で無効。VRChat OSC画面で有効化し「適用して保存」。OSCQuery自動検出は初期状態でON。[vrchat_osc](https://github.com/minetake01/vrchat_osc)でmDNSのOSC/OSCQueryサービスを対応づけ、HTTPの生存確認後に検出されたUDP宛先へ送ります。再確認は5秒ごと、HTTPタイムアウトは2秒。VRChat未起動・OSC無効時は検出待ちとなります。自動検出をOFFにすると手動宛先（初期値 `127.0.0.1:9000`）を使用。検出は受信パラメータの適用保証ではないため、アバター側のパラメータ設定も必要です。4Hz（250ms周期）で最新状態を再送します。Sleep Engineの評価は1Hzです。

VRCOSC Heartrateの既存9パラメータを維持。Sleepは以下を送ります。

| パラメータ（`/avatar/parameters/VRCOSC/Sleep/`以下） | 型 | 意味 |
|---|---|---|
| Enabled | Bool | 出力が有効 |
| Tracking | Bool | 現在判定できる |
| Probability | Float | 0〜1の暫定スコア |
| Asleep | Bool | 継続条件を満たした睡眠状態 |
| Estimated | Bool | HRのみの擬似判定 |
| MeasurementAvailable | Bool | RRベースの睡眠計測が可能 |

Sleep/Tracking=falseではAsleep=falseを「覚醒」と解釈せず、判定不能として扱ってください。

RESTはSettingsまたは`--api`で有効化。localhost限定、既定8791番。

```text
GET /api/v1/sleep/status
GET /api/v1/physiology/latest
GET /api/v1/physiology/history?since=<Unix milliseconds>
GET /api/v1/sleep/sessions
GET /api/v1/sleep/sessions/{id}
GET /api/v1/settings
```

`/sleep/status`は `asleep` / `tracking` / `mode` / `estimated` / `measurement_available` / `probability` / `updated_at` / `source_timestamp` を返します。判定不能なら`asleep`と`probability`はnullです。
`mode`は `unavailable` / `hr_only` / `hr_rr`。既存latest・Unix IPCにもmodeと`rr_available`を含めています。

Unix IPCは`--ipc /path/to/socket`で有効化。1行1SnapshotのJSON。Windows Named Pipe / Shared Memory / OSCQueryはまだ未対応です。

## テスト

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p nemuribeat
python3 tests/smoke.py target/debug/nemuribeat
```

smokeはHR＋RRとHRのみの96秒テストを同時実行します。実機・Pulsoidアカウント不要です。アプリ本体はPython不要です。
`--demo-sleep`は睡眠寄りの固定HRを生成する試験用モードです。通常起動には影響しません。

実Pulsoidトークン・Coospo実機・VRChat実接続・Windows実行・一晩の計測は未検証です。

## ロゴ

指定のnemuribeat-moon-pulse-v0.4を `apps/desktop/assets/branding` に同梱。eguiウィンドウアイコン、サイドバー、Windows EXEリソース、macOSバンドルのICNSに反映。Windowsのアイコン埋め込みにはMSVC付属のリソースコンパイラが必要です。
