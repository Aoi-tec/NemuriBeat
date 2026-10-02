# 更新: 2026-10-02

追加仕様は `NEMURIBEAT.md` を優先してください。製品名はNemuriBeatです。

# Architecture.mdとの対応

設計書原本は変更していません。ここにある「実装済み」はコードの有無を示し、実機での完成認定を意味しません。

| フェーズ | 実装済み | 残件 |
|---|---|---|
| 1 Acquisition | 標準BLE HRスキャン・選択・接続・HR/RRパース・再接続・Raw保存、Pulsoid OAuth Device Flow / WebSocket入力 | Coospo/Polar実機、バッテリー情報、長時間の接続試験 |
| 2 Realtime Core | Snapshot / bounded Ring / Event Bus / Feature cache / HRV / 品質 | メーカー別RRアーティファクト評価、実センサーでの遅延測定 |
| 3 GUI | 全 7 画面（Dashboard / Sleep / Realtime / History / VRChat OSC / Sensors / Settings）、日本語 UI、ダーク / ライト、縦軸が自動で伸縮するグラフ（egui_plot）、保存状態表示 / tracing | テーマ設定の保存、runtime が出すメッセージ本体の日本語化（いまは UI 側で置き換え）、ログファイルローテーション、OS起動時実行 |
| 4 Sleep | RR条件成立時の自動Session開始・手動/自動終了、暫定Probability、EMA、ヒステリシス、グラフ、日跨ぎ履歴、個人平均Baseline | 外部正解ラベルによる校正、安静/夜間Baseline分離、睡眠規則性集計、欠測率詳細、部分区間の厳密な時間加重統計 |
| 5 VRChat | 全Heartrate/Sleepパラメータ、OSCQuery/mDNS自動検出・HTTP生存確認、手動UDP宛先、状態再送、停止時の解除送信 | Avatarパラメータの型・適用検証、VRChat実機、Beatバーストの実アバター確認 |
| 6 Integration | 同一プロセスwatch、Unix socket、read-only REST | Shared Memory ABI、Windows named pipe、WebSocket要否評価 |
| 7 ML / Emotion | Consumer境界のみ | データセット、学習、ONNX、Emotion Engineすべて未実装 |

## 実装上の判断

- GUIはRustで完結するeframe/egui。RESTをGUIのリアルタイム経路に使わない。
- Coospo/Polarは標準Heart Rate Serviceを使う機器をGeneric adapterで扱う。
- RawはBLE通知単位に全RR配列を保存。HRのみの通知も保存し、RRを捏造しない。
- RRの生理範囲300–2000ms、隣接変化25%を暫定除外基準とする。除外点を跨いだ差をRMSSDに混ぜない。
- 睡眠推定に採用するBaselineは過去に保存された有効特徴量のみ。最低30分必要。これは暫定の個人平均で、安静HRの同定は未完了。
- 保存障害の際にRealtimeを止めないためキューに上限を設け、あふれを明示する。無限メモリでの欠損ゼロは約束しない。
- OSCQuery検出状態と手動UDP送信状態を分離。UDP送信成功からVrchatConnectedを発火させない。
- SocketはShared Memoryの代替完成扱いにしない。固定レイアウト・バージョン・同期方式が必要なShared Memoryは別工程。

## 次の受入試験

1. macOS / WindowsでCoospoを接続し、HR、複数RR、接触不良、切断、再接続を記録。
2. 8時間以上の連続記録でRaw通知数、欠損、DBサイズ、メモリ量、終了後のセッション整合性を検証。
3. VRChatアバターに各パラメータを登録し、型・正規化・Beat・再起動時復帰を検証。
4. 実VRChatでOSCQuery検出・再起動・複数NIC環境とアバター適用を確認。
5. 正解付き睡眠データで暫定Probabilityを評価してから、校正・Baselineの改良に進む。

## 参照した互換仕様

- https://docs.vrchat.com/docs/osc-avatar-parameters
- https://github.com/hizkifw/HeartOSC/blob/main/app/src/main/java/red/kitsu/heartosc/VrcoscHeartrateParameters.kt
- https://docs.rs/btleplug/0.11.8/btleplug/
- https://docs.rs/eframe/0.33.3/eframe/
- https://docs.rs/egui_plot/0.34.1/egui_plot/

HeartOSC参照に合わせ、Normalised=HR/240、Average=直近10秒平均、Units/Tens/Hundreds=各桁/10。ネットワークで届くBeatはBLE通知に由来し、完全な拍動時刻の再生を保証しない。
