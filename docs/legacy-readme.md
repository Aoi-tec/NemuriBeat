```markdown
```
```markdown
# VRC-Health-Check

目的:
- Bluetooth 心拍計（BLE）からRR間隔と心拍数を自動取得し、OSC（Open Sound Control）で出力する。
- 取得したデータを Apple HealthKit / Google Fit に書き込めるようにする。

技術方針（高レベル）:
- BLE 周辺機器（Heart Rate Service, UUID: 0x180D）から Heart Rate Measurement キャラクタリスティクスを購読して、心拍数とRR間隔を取得する。
- 取得したRR値および心拍数をOSCメッセージにマッピングしてUDPで送信する（例: /vrc/heartrate, /vrc/rr）。
- iOS では HealthKit を用いてサンプル（HKQuantitySample, HKCorrelation など）として書き込む。
- Android では Google Fit APIs（Recording API / Sensors API / History API）を用いて書き込む。

実装メモ（要点）:
- BLE:
	- 探索 -> 接続 -> Heart Rate Measurement キャラクタリスティクスのNotifyを有効化。
	- Heart Rate Measurement のペイロードからFlagsとRR間隔（16bit単位、単位: 1/1024秒）をパース。
	- RRはミリ秒へ変換して扱う。
- OSC:
	- ライブラリ例: osc-js, python-osc, liblo など。
	- 送信例: /vrc/heartrate <bpm:int>, /vrc/rr <rr_ms:float>
- HealthKit (iOS):
	- HKQuantityTypeIdentifierHeartRate, HKQuantityTypeIdentifierHeartRateVariabilitySDNN（必要なら）を使用。
	- 心拍数は HKQuantity(unit: .count().unitDivided(by: .minute())) で書き込む。
	- RR間隔は直接の型がない場合、心拍変動指標（SDNN, RMSSD）に変換して保存。
- Google Fit (Android):
	- DataType.TYPE_HEART_RATE_BPM を使い、DataPoint にタイムスタンプと値を入れて History API へ書き込む。

プライバシーと権限:
- iOS: HealthKit の書き込み権限を明示的にユーザから取得すること（NSHealthShareUsageDescription 等）。
- Android: Google Fit 用の OAuth スコープを要求（fitness.activity.write, fitness.heart_rate.write など）。

動作フロー（簡潔）:
1. BLEデバイスをスキャンして選択
2. 接続してNotify購読開始
3. パースして内部バッファへ保存
4. OSCで外部へ送信
5. 必要に応じて HealthKit / Google Fit に書き込み

次のステップ（実装タスク）:
- BLE 接続と HRM パーサの実装（クロスプラットフォームなら Flutter / React Native / Kotlin Multiplatform を検討）
- OSC 出力の実装とテスト（ローカルUDPで確認）
- HealthKit / Google Fit 書き込みの実装と権限フローの実装

参考:
- Heart Rate Service (GATT): https://www.bluetooth.com/specifications/gatt/services/
- HealthKit: https://developer.apple.com/documentation/healthkit
- Google Fit: https://developers.google.com/fit
```
