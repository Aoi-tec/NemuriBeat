# Architecture.md

## 1. 概要

本ソフトウェアは、Coospo等のBLE心拍センサーから取得した心拍数（HR）・RR Intervalを中心に、生体情報をリアルタイム処理・保存・可視化し、単体の睡眠解析ソフトとして利用できることを第一目的とする。

同時に、将来的な感情推定プログラム、VRChat、TTS、デスクトップキャラクター等へ生体情報を低遅延で提供できる **Realtime Physiological Middleware / Physio Core** として設計する。

主な用途は以下。

- 心拍・RR Intervalのリアルタイム取得
- HRV等の内部解析
- 睡眠 / 覚醒確率の推定
- 一晩の睡眠グラフ生成
- 7日 / 30日 / 90日等の履歴表示
- 個人Baselineとの比較
- VRCOSC互換の心拍OSC出力
- VRChat向け睡眠確率OSC出力
- 将来の感情推定エンジンへの低遅延データ提供
- 生データの長期保存と再解析

---

## 2. 設計方針

### 2.1 単体ソフトとして完結させる

本ソフトは「外部AIへデータを渡すだけのCollector」にはしない。

単体で以下を完結できるものとする。

- センサー接続
- リアルタイム表示
- 睡眠推定
- 日次睡眠レポート
- 長期履歴
- VRChat OSC出力
- データ保存
- 設定管理

将来のEmotion Engine等は追加Consumerとして接続する。

### 2.2 Realtime処理とExternal APIを分離する

数ms単位で利用される内部データをREST API経由で取得させない。

Realtime処理はメモリ上で完結させる。

```text
Sensor
  ↓
Realtime Core
  ├─ Latest Snapshot
  ├─ Ring Buffer
  ├─ Event Bus
  └─ Feature Cache
       │
       ├─ Sleep Engine
       ├─ OSC Adapter
       ├─ GUI
       ├─ Emotion Interface
       └─ Async Storage Worker
```

REST APIは以下の用途に限定する。

- 履歴取得
- 設定
- セッション情報
- デバッグ
- 外部ツールからの低頻度アクセス

### 2.3 ProducerとConsumerの周期を分離する

```text
Coospo
  │ Beat / BLE Notify
  ▼
Realtime Core
  │ 最新状態を保持
  ├─ GUI                10～30 Hz
  ├─ OSC                1～20 Hz
  ├─ Sleep Engine       0.2～1 Hz
  └─ Emotion Engine     必要に応じて高頻度参照
```

Consumerは最新Snapshotを読むだけとし、Sensorへの再問い合わせは行わない。

---

## 3. 全体アーキテクチャ

```text
┌───────────────────────────────────────────────────────────────┐
│                        Desktop Application                    │
│                                                               │
│  ┌─────────────────┐                                          │
│  │ Sensor Adapters │                                          │
│  │ BLE / Coospo    │                                          │
│  └────────┬────────┘                                          │
│           │ HR / RR                                           │
│           ▼                                                   │
│  ┌───────────────────────────────────────────────┐            │
│  │              Realtime Physio Core             │            │
│  │                                               │            │
│  │  Latest Snapshot                              │            │
│  │  RR Ring Buffer                               │            │
│  │  Feature Cache                                │            │
│  │  Event Bus                                    │            │
│  └───────┬──────────────┬──────────────┬─────────┘            │
│          │              │              │                      │
│          ▼              ▼              ▼                      │
│   Feature Engine   Sleep Engine    OSC Adapter                │
│          │              │              │                      │
│          │              │              ├─ VRCOSC Heartrate    │
│          │              │              ├─ VRCOSC Sleep        │
│          │              │              └─ OSCQuery / VRChat   │
│          │              │                                     │
│          └──────┬───────┘                                     │
│                 ▼                                             │
│          Async Storage Worker                                 │
│                 │                                             │
│                 ▼                                             │
│              SQLite                                           │
│                                                               │
│  ┌────────────────────────┐   ┌────────────────────────────┐  │
│  │ GUI                    │   │ External Integration       │  │
│  │ Dashboard              │   │ Shared Memory             │  │
│  │ Sleep                  │   │ Named Pipe / Local Socket │  │
│  │ Realtime               │   │ REST / WebSocket          │  │
│  │ History                │   │ Future Emotion Engine      │  │
│  │ VRChat OSC             │   └────────────────────────────┘  │
│  │ Sensors                │                                    │
│  │ Settings               │                                    │
│  └────────────────────────┘                                    │
└───────────────────────────────────────────────────────────────┘
```

---

## 4. 実装言語

### 4.1 メインアプリ

基本実装：

```text
Rust
```

Rust側で以下を担当する。

- BLE通信
- Sensor Adapter
- Realtime Core
- Ring Buffer
- HRV計算
- Sleep Engine
- Storage
- OSC
- IPC
- REST
- GUIバックエンド

現段階ではCUDAは必要としない。

### 4.2 将来の機械学習

```text
Python
 + PyTorch
 + CUDA
      ↓
     ONNX
      ↓
Rust Application
 + ONNX Runtime
```

アプリ本体をPythonへ依存させない。

---

## 5. Sensor Adapter

責務：

- BLE Device Discovery
- 接続
- 再接続
- Heart Rate Service購読
- HR取得
- RR Interval取得
- Sensor Connection State管理
- 時刻付与

Sensor Adapterは特定メーカー依存にしない。

```text
SensorAdapter Trait
├─ CoospoBleAdapter
├─ PolarAdapter
├─ GenericBleHRAdapter
└─ OtherSensorAdapter
```

---

## 6. Realtime Physio Core

### 6.1 Latest Snapshot

```rust
PhysioSnapshot {
    sequence_id,
    timestamp,
    heart_rate,
    last_rr_ms,
    rmssd_fast,
    signal_quality,
    sensor_connected,
}
```

高速ConsumerはDBやHTTPを経由せずSnapshotを参照する。

### 6.2 Ring Buffer

```text
RR Buffer           10～30分
Fast Feature Buffer 1～5分
Display Buffer      数時間
```

### 6.3 Event Bus

```text
SensorConnected
SensorDisconnected
HeartBeat
SleepSessionStarted
SleepSessionEnded
SleepProbabilityChanged
BaselineUpdated
StorageError
VrchatConnected
VrchatDisconnected
```

---

## 7. Feature Engine

初期実装：

- Mean HR
- Min HR
- Max HR
- RR Artifact除去
- RMSSD
- SDNN
- pNN50
- HR Trend
- HR Slope
- Baselineとの差
- Signal Quality

必要に応じて追加：

- LF
- HF
- LF/HF
- Respiratory estimate
- Accelerometer feature
- SpO2
- Skin temperature

これらは内部解析用であり、必ずしもOSCへ公開しない。

---

## 8. Sleep Engine

### 8.1 出力

```text
Sleep Probability
0.0 ～ 1.0
```

意味：

```text
0.0 = 覚醒寄り
0.5 = 判定が曖昧
1.0 = 睡眠寄り
```

内部ではHR、RR、HRV、Baseline、時間的推移等を利用してよい。

### 8.2 Tracking State

センサー切断やデータ品質不足の場合、

```text
Tracking = false
```

とする。

これにより `Probability = 0.0` が「覚醒」なのか「判定不能」なのかを区別できる。

---

## 9. 睡眠セッション

```text
SleepSession
├─ session_id
├─ bed_time
├─ estimated_sleep_start
├─ final_wake_time
├─ session_end
├─ sleep_duration_estimated
├─ avg_hr
├─ min_hr
├─ median_rmssd
├─ awakening_candidates
└─ baseline_delta
```

日付ではなくSession IDを主キーとする。

---

## 10. 睡眠グラフ

### 10.1 日次画面

表示候補：

- Sleep Probability
- Heart Rate
- HRV
- Estimated Awake / Sleep-like periods
- Wake candidates
- Sensor Quality

```text
00:00                                         07:00

Sleep Probability
1.0 ┤      ┌─────────────────┐     ┌─────────
0.5 ┤  ────┘                 └─────┘
0.0 ┤──
    └────────────────────────────────────────

Heart Rate
80 ┤
70 ┤ ╲                  ╱╲
60 ┤  ╲____      ______╯  ╲____
50 ┤       ╲____╯
   └─────────────────────────────────────────
```

### 10.2 表示用データ

```text
Raw RR
  ↓
Feature Aggregation
  ↓
1分単位 Display Feature
  ↓
Sleep Graph
```

Rawは研究・再解析用として保持する。

---

## 11. 長期履歴

表示期間：

- 7日
- 30日
- 90日
- 全期間

表示候補：

- 推定睡眠時間
- 就寝時刻
- 起床時刻
- 平均HR
- 最低HR
- RMSSD
- Baseline差
- Sleep Probability統計
- 睡眠規則性

---

## 12. Storage

初期DB：

```text
SQLite
```

### 12.1 raw_measurements

```text
timestamp
sensor_id
heart_rate
rr_ms
quality
sequence_id
```

### 12.2 physio_features_1m

```text
timestamp
hr_mean
hr_min
hr_max
rmssd
sdnn
sleep_probability
quality
```

### 12.3 sleep_sessions

```text
session_id
started_at
estimated_sleep_at
final_wake_at
ended_at
estimated_sleep_minutes
avg_hr
min_hr
median_rmssd
awakening_count
```

### 12.4 settings

```text
key
value
updated_at
```

---

## 13. 非同期保存

```text
Realtime Core
      ↓
Storage Queue
      ↓
Storage Worker
      ↓
SQLite Batch Write
```

DB停止や一時的な遅延がRealtime処理へ波及しない構造にする。

---

## 14. External Integration

### 14.1 同一プロセス

```text
Atomic / Watch Channel / Ring Buffer
```

### 14.2 別プロセス高速連携

```text
Shared Memory
```

概念：

```text
Rust Physio Core
       │
       ▼
SharedPhysioState
       │
       ▼
Python / CUDA Emotion Engine
```

### 14.3 別プロセス制御系

```text
Named Pipe
Local Socket
```

### 14.4 REST API

```text
GET /api/v1/physiology/latest
GET /api/v1/physiology/history
GET /api/v1/sleep/sessions
GET /api/v1/sleep/sessions/{id}
GET /api/v1/settings
```

高頻度Consumerには使用しない。

---

## 15. VRChat / OSC

```text
Physio Core
    ↓
OSC Mapper
    ↓
VRCOSC Compatible Parameters
    ↓
VRChat
```

VRChat Avatar Parameterへの送信は、

```text
/avatar/parameters/<ParameterName>
```

形式とする。

---

## 16. VRCOSC Heartrate互換

既存VRCOSC Heart Rate対応アバターとの互換性を保つ。

| Parameter | Type |
|---|---|
| `VRCOSC/Heartrate/Connected` | Bool |
| `VRCOSC/Heartrate/Enabled` | Bool |
| `VRCOSC/Heartrate/Value` | Int |
| `VRCOSC/Heartrate/Normalised` | Float |
| `VRCOSC/Heartrate/Average` | Int |
| `VRCOSC/Heartrate/Beat` | Bool |
| `VRCOSC/Heartrate/Units` | Float |
| `VRCOSC/Heartrate/Tens` | Float |
| `VRCOSC/Heartrate/Hundreds` | Float |

例：

```text
/avatar/parameters/VRCOSC/Heartrate/Value
/avatar/parameters/VRCOSC/Heartrate/Beat
```

---

## 17. Sleep OSC

Namespace：

```text
VRCOSC/Sleep/*
```

| Parameter | Type | 内容 |
|---|---|---|
| `VRCOSC/Sleep/Enabled` | Bool | Sleep Engine / OSC出力が有効 |
| `VRCOSC/Sleep/Tracking` | Bool | 現在睡眠推定が可能 |
| `VRCOSC/Sleep/Probability` | Float | 睡眠確率 0.0～1.0 |

実OSC：

```text
/avatar/parameters/VRCOSC/Sleep/Enabled
/avatar/parameters/VRCOSC/Sleep/Tracking
/avatar/parameters/VRCOSC/Sleep/Probability
```

### 17.1 Probability

```text
0.0 = 覚醒寄り
0.5 = 判定が曖昧
1.0 = 睡眠寄り
```

VRChat側で任意に閾値を設定できる。

例：

```text
Probability < 0.40
    → Awake

0.40 ～ 0.70
    → Transition

Probability > 0.70
    → Asleep
```

### 17.2 Tracking

```text
Tracking = false
```

となる例：

- Sensor disconnected
- RR不足
- Signal quality不足
- Sleep Engine初期化中

---

## 18. OSC更新周期

推奨初期値：

```text
Heartrate/Value
    Sensor更新時

Heartrate/Beat
    Beat Event時

Heartrate/Normalised
    Sensor更新時

Sleep/Probability
    最大1～5 Hz
    または一定値以上変化した場合

Sleep/Tracking
    状態変化時 + 定期再送

Sleep/Enabled
    起動時 / 設定変更時 / 再接続時
```

Sleep Probabilityは数ms周期で変化させる必要はない。

---

## 19. OSC再送

```text
状態変化 → 即時送信
+
一定周期 → 現在値を再送
```

VRChat再起動、OSC再接続、Avatar変更、パケットロスから復帰しやすくする。

---

## 20. OSCQuery

VRChatとの接続にはOSCQuery対応を推奨する。

目的：

- VRChatインスタンス検出
- OSC Port Discovery
- Parameter Discovery
- 手動IP / Port設定の削減

固定Port指定もFallbackとして残す。

```text
VRChat Receive: 9000
VRChat Send:    9001
```

---

## 21. GUI

```text
Dashboard
Sleep
Realtime
History
VRChat OSC
Sensors
Settings
```

### Dashboard

- Sensor Status
- Current HR
- Current RR
- Sleep Tracking
- Sleep Probability
- Today Sleep Summary
- VRChat OSC Status

### Sleep

- Sleep Probability graph
- HR graph
- HRV graph
- Sleep / Awake estimate
- Session summary
- Wake candidates
- Sensor quality

### Realtime

- HR
- RR
- HRV
- Sleep Probability
- Signal Quality

### History

- 7 days
- 30 days
- 90 days
- All

### VRChat OSC

```text
Enable OSC

OSCQuery       ON/OFF
Target         Auto / Manual
Host
Port

VRCOSC Heartrate Compatibility ON/OFF
Sleep Output                    ON/OFF

Current:
Connected
HR
Sleep Tracking
Sleep Probability
```

### Sensors

- Device Scan
- Device Select
- Connect
- Auto Reconnect
- Sensor Quality
- Battery（取得可能な場合）

### Settings

- Data retention
- Sleep Engine
- Baseline period
- OSC
- API
- Shared Memory
- Logging
- Startup
- Theme

---

## 22. Sleep Probabilityの平滑化

瞬間的なHR変動でProbabilityが振動しないよう平滑化する。

候補：

```text
EMA
Moving Average
Temporal Window Model
```

内部状態表示用にはHysteresisを利用可能。

```text
Sleep Entry:
Probability >= 0.75 が一定時間継続

Wake:
Probability <= 0.40 が一定時間継続
```

OSCには連続値Probabilityを出す。

---

## 23. Baseline

絶対値より個人内変化を重視する。

Baseline候補：

```text
直近14日
直近30日
```

保持候補：

- Resting HR
- Night HR
- RMSSD
- Sleep timing
- Sleep duration

---

## 24. Sensor追加への対応

将来的に以下を追加可能とする。

```text
Accelerometer
SpO2
Skin Temperature
Respiration
PPG
ECG
```

Sensor追加によってOSC Specificationを変更する必要はない。

```text
内部入力増加
      ↓
推定精度改善
      ↓
VRCOSC/Sleep/Probability
```

を維持する。

---

## 25. 感情推定への将来対応

Emotion Engineは本システムとは別Consumerとして設計する。

```text
Physio Core
    │
    ├─ Sleep Engine
    ├─ OSC / VRChat
    ├─ GUI
    └─ Shared Memory
            ↓
       Emotion Engine
```

Emotion Engineが高頻度で値を要求してもREST APIを使用しない。

---

## 26. データの責務分離

### Raw

```text
timestamp
HR
RR
sensor metadata
quality
```

### Feature

```text
RMSSD
SDNN
HR trend
Sleep Probability
etc.
```

### Summary

```text
Sleep Session
Daily Summary
Baseline
Trend
```

---

## 27. 障害時動作

### Sensor切断

```text
Connected = false
Tracking = false
```

### DB書き込み失敗

Realtime Coreは継続する。

```text
Realtime
   ↓
Memory Queue
   ↓
Retry
```

### VRChat未起動

OSC AdapterのみDisconnected状態。

Sleep Engine・記録・GUIは通常動作を継続する。

### Emotion Engine未起動

Consumer不在によってCore処理を停止しない。

---

## 28. Rust Workspace案

```text
physio-app/
│
├─ Cargo.toml
│
├─ apps/
│   └─ desktop/
│
├─ crates/
│   ├─ physio-core/
│   ├─ sensor-core/
│   ├─ sensor-ble/
│   ├─ feature-engine/
│   ├─ sleep-engine/
│   ├─ storage/
│   ├─ osc-vrchat/
│   ├─ ipc/
│   ├─ api/
│   └─ shared-types/
│
├─ migrations/
│
├─ docs/
│   └─ Architecture.md
│
└─ tests/
```

---

## 29. 開発フェーズ

### Phase 1 — Acquisition

- Generic BLE HR接続
- Coospo接続
- HR取得
- RR取得
- Reconnect
- Raw保存

### Phase 2 — Realtime Core

- Latest Snapshot
- Ring Buffer
- Event Bus
- Feature Engine
- Signal Quality

### Phase 3 — Standalone GUI

- Dashboard
- Realtime
- Sensor Settings
- Storage
- Logging

### Phase 4 — Sleep

- Sleep Session
- Sleep Probability v1
- Sleep graph
- Daily Summary
- History
- Baseline

### Phase 5 — VRChat

- OSC Output
- VRCOSC Heartrate compatibility
- `VRCOSC/Sleep/Enabled`
- `VRCOSC/Sleep/Tracking`
- `VRCOSC/Sleep/Probability`
- OSCQuery
- Reconnect / Resend

### Phase 6 — External Integration

- Shared Memory
- Named Pipe / Local Socket
- REST API
- WebSocket必要性の再評価

### Phase 7 — ML / Emotion

- Dataset作成
- Python / PyTorch Training
- Model Evaluation
- ONNX Export
- Rust Inference
- Emotion Engine接続

---

## 30. 初期完成条件

本ソフトのv1完成条件：

- CoospoからHR / RRを安定取得できる
- 一晩のRawデータを欠損管理付きで保存できる
- HR / HRV推移を表示できる
- Sleep Probabilityを生成できる
- 日次睡眠グラフを表示できる
- 過去履歴を確認できる
- VRCOSC Heartrate互換出力ができる
- `VRCOSC/Sleep/Probability`をVRChatへ出力できる
- Sensor切断時に`Tracking=false`へ移行できる
- OSCが停止しても睡眠記録が継続する
- External Consumer不在でも単体アプリとして完全動作する

---

## 31. 最終的な位置付け

本ソフトは、

> **睡眠トラッカーを内包したリアルタイム生体情報プラットフォーム**

として設計する。

```text
                    ┌─ Sleep Tracker / Analyzer
                    │
Sensor → Physio Core├─ VRChat / VRCOSC
                    │
                    ├─ History / Dashboard
                    │
                    ├─ Future Emotion Engine
                    │
                    ├─ Future TTS
                    │
                    └─ Future Desktop Character
```

睡眠機能だけでも独立して完成したアプリとして利用できる。

その内部データを、将来必要になった時点でEmotion Engineや他のリアルタイムシステムへ渡す構造とする。

---

## 32. 参考仕様

- VRCOSC Module / Parameter convention  
  https://vrcosc.com/docs/v1/module-creation

- VRChat OSC Avatar Parameters  
  https://docs.vrchat.com/docs/osc-avatar-parameters

- VRChat OSCQuery  
  https://docs.vrchat.com/docs/oscquery

- VRChat OSC Overview  
  https://docs.vrchat.com/docs/osc-overview

- VRCOSC Bluetooth Heartrate互換パラメータ例  
  https://github.com/hizkifw/HeartOSC
