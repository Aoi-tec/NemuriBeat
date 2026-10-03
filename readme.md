# NemuriBeat

BLE / HypeRateの心拍を取得し、リアルタイム表示・睡眠状態出力・RRを用いた睡眠記録を行うRust製デスクトップアプリです。
既存のデザイン（テーマ、フォント、チャート）を維持しています。設計原本は `Architecture.md`、追加仕様は `docs/NEMURIBEAT.md` を参照してください。

OyasumiVRの睡眠モード連携も対応しています。Settingsで有効にし、OSCQueryで接続先を自動検出します。導入方法は [docs/OYASUMIVR.md](docs/OYASUMIVR.md) を参照してください。

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
| HypeRate、RRなしBLE | HR / 擬似睡眠 | グレーアウト | 作成しない | 擬似睡眠の状態 |
| 切断、HR無受信 | 判定不能 | 無効 | 記録終了 | Tracking=false、Asleep=false |

- Sensors → BLEスキャン・接続、またはHypeRate IDとAPIキーを入力して接続。
- HypeRate APIキーはメモリ内だけに保持し、DB・ログ・Webページ・RESTに保存/公開しません。IDのみ「適用して保存」で保持できます。
- 入力の切替は先に「入力を切断 / 接続を中止」。再接続も停止できます。
- RR対応機器でもRRが5秒以上届かなくなるとHRのみへ切替。HRも5秒以上無受信なら判定不能。
- RR睡眠はSleepECG公式GRUを使用。HRのみは暫定ルールの擬似状態であり、睡眠計測として保存しません。VR環境での判定精度は未検証です。

### HypeRate

[公式API案内](https://www.hyperate.io/api.html)でAPIキーを申請し、HypeRateアプリに表示されるIDとともにSensorsへ入力してください。
[公開API仕様](https://github.com/alexholzreiter/HypeRate-Websocket-API/blob/main/docs/WEBSOCKET_API.md)の `wss://app.hyperate.io/ws/:deviceId?token=...` へ接続し、`hr:ID` に参加して `hr_update` を受信します。15秒ごとにkeepaliveを送り、切断は1〜30秒で再接続。認証拒否は停止します。接続だけでは受信扱いにせず、心拍が届いた時点で有効になります。

`timestamp`は受信時のUnixミリ秒。公開仕様に測定時刻/RRがないため `source_timestamp=null`、RR空とし、HRのみの擬似睡眠として扱います。同じBPMの新着通知も有効な測定として受信します。

```sh
# HYPERATE_ID / HYPERATE_API_KEY をプロセス環境に設定したheadless起動
cargo run -p nemuribeat -- --headless --hyperate
```

### 常駐とBuilt-in Web Browser

Windowsのトレイ / macOSのメニューバーにNemuriBeatアイコンを表示します。ウィンドウの「閉じる」は初期設定で非表示のみとし、計測・OSC・Web配信を継続。アイコンのメニューから表示へ戻すか「終了」で保存して停止できます。Settingsで閉じても常駐する動作を変更できます。`--start-hidden` はトレイ作成後に画面を隠して起動します。トレイ作成失敗時や未対応OSでは画面を隠さず、閉じる操作で通常終了します。

XSOverlayとOVR Toolkitは、共通のHTMLページをBuilt-in Web Browserで開く方式です。

```text
http://127.0.0.1:8791/overlay/heart-rate
```

Settingsの「Web心拍表示」は初期状態でON。ポートを変更した場合はURLにも同じポートを指定します。REST APIをOFFにしてもWeb心拍表示は使えます。心拍用JSONと静的ページのみを配信し、履歴や設定APIは有効になりません。サーバーは127.0.0.1のみで待ち受けます。

- XSOverlay: Web Browserを開き、上記URLを入力。対応版のBuilt-in Web Browserを使用してください。
- OVR Toolkit: Built-in Web Browser / URLを指定するブラウザ機能で同じURLを開きます。インストール済み版にネイティブブラウザ入口がない場合は公式Custom URLブラウザ機能で同じページを開けます。
- 心拍のみの小さな表示: URL末尾に `?compact=1`。
- 背景透過: `?transparent=1`。心拍のみ＋透過は `?compact=1&transparent=1`。背景透過の可否は各オーバーレイのブラウザ設定にも依存します。

500msごとに読み込み（同時リクエストなし）、受信停止/通信失敗時には古いBPMを消して判定不能にします。上流から供給される測定値以上の時間分解能は作りません。

背景の一本の線はBPMの周期で脈打つ心電図風アニメーションです（75 BPMなら約0.8秒に1回）。実際の心電図・拍動時刻ではなく、BPMから生成した擬似波形です。受信停止時は波形を消して水平線に戻します。コンパクト・透過表示にも対応し、非表示中やOSの動きを減らす設定ではアニメーションを停止します。

### 自動睡眠記録

RR睡眠推定は有効RR 30個・品質0.8以上・SleepECG公式GRUの有効な結果が必要です。専用Python 3.12環境を導入し、Settingsの「睡眠モデル用Python実行ファイル」へ絶対パスを設定してください。[導入と睡眠仕様](docs/SLEEP_GRU_OPENVR.md)。公式GRUは連続RR約540秒の蓄積と270秒の先読みが必要なため、結果は約4〜5分遅れます。モデルの未準備・停止で古いRRルールへは戻らず判定不能になります。旧ルールはデモ専用です。

OpenVRのHMD静止・装着条件は初期ON。30秒の加速度の大きさの移動平均が厳密に0、取得可能な近接センサーが未装着、または追跡不能なら棄却します。SteamVRを使わない計測ではSettingsでHMD条件をOFFにできます。静止閾値の実機校正は未完了です。
Probabilityが0.75以上で設定時間（初期値5分、60〜1800秒）続くと睡眠と判定し、その時点でSessionを作成・保存します。継続判定の開始時刻を入眠候補時刻として、直前の集計も記録へ取り込みます。

覚醒側はProbabilityが0.40以下で2分継続。RR喪失・切断・判定不能では記録を終了します。手動終了後は一度覚醒側へ戻るまで再記録しません。Settingsで自動記録を無効にすることもできます。

HRのみは基準心拍（初期値60 bpm）と傾きを使う暫定ルール＋EMAで擬似睡眠を生成します。継続時間の条件は同じです。単なる低心拍を睡眠と判定する可能性があるため、実睡眠記録と混ぜません。

SleepとHistoryは正午〜翌正午を1日とする集約表示です。切断や再起動で分かれたSessionを同じ日にまとめ、記録間の空白は睡眠時間に加えません。元の区間はDBと従来APIに保持します。日付区切りはSettingsで変更可能です。

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

メモリには20分/最大6000通知のRRバッファ、約30分/最大1805点の表示キャッシュ、最大36件の自動記録前集計を保持。履歴は選択した睡眠日の区間だけ読込みます。セッション中央値は固定長の1ms単位ヒストグラムで集計します。時間に比例してメモリを増やさず、長期のRawはDBへ保存します。

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
GET /api/v1/sleep/days
GET /api/v1/sleep/days/YYYY-MM-DD
GET /api/v1/settings
```

`/sleep/status`は `asleep` / `tracking` / `mode` / `estimated` / `measurement_available` / `probability` / `updated_at` / `source_timestamp` を返します。判定不能なら`asleep`と`probability`はnullです。
`/sleep/status`とSnapshotには`model`（推定対象／受信時刻／準備状態）と`hmd`（装着・平均加速度・棄却等）も含みます。`/sleep/days`は1日単位のsummaryと元のsegmentsを返します。
`mode`は `unavailable` / `hr_only` / `hr_rr`。既存latest・Unix IPCにもmodeと`rr_available`を含めています。

Unix IPCは`--ipc /path/to/socket`で有効化。1行1SnapshotのJSON。Windows Named Pipe / Shared Memoryはまだ未対応です。OSCQueryは実装済みです。

## テスト

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p nemuribeat
python3 tests/smoke.py target/debug/nemuribeat
python3 tests/oscquery_smoke.py target/debug/nemuribeat
node tests/overlay-web.js
python3 tests/sleep_workers.py
# 専用SleepECG環境: python tests/sleep_workers.py --model
```

smokeはHR＋RRとHRのみの96秒テストを同時実行します。実機・HypeRateアカウント不要です。HR表示・デモにはPython不要です。実RRのGRU推論は専用Python環境が必要です。
`--demo-sleep`は睡眠寄りの固定HRを生成する試験用モードです。通常起動には影響しません。

実HypeRate配信・Coospo実機・VRChat実接続・Windows実行・一晩の計測は未検証です。

未完了一覧は[Task.MD](Task.MD)、新しい睡眠仕様は[SleepECG / OpenVR仕様](docs/SLEEP_GRU_OPENVR.md)を参照してください。

## ロゴ

指定のnemuribeat-moon-pulse-v0.4を `apps/desktop/assets/branding` に同梱。eguiウィンドウアイコン、サイドバー、Windows EXEリソース、macOSバンドルのICNSに反映。Windowsのアイコン埋め込みにはMSVC付属のリソースコンパイラが必要です。
# VRC-Health-Check
