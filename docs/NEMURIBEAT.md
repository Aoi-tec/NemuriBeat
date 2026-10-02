# 2026-10-02 追加要件と実装

製品名はNemuriBeat。既存テーマ・日本語フォント・チャート構成を維持し、入力モードに応じてUIを切り替える。

## 入力

Pulsoidは新規sensor-pulsoid crateのSensorAdapter。公式WebSocketを使用し、HRのみのMeasurementをCoreへ渡す。受信時計と送信元時計を分離し、RRは捏造しない。メッセージサイズ上限64KiB、重複/逆順timestampを抑止、接続失敗は指数バックオフ。401/403では停止する。WS接続だけでセンサー接続扱いにはせず、新しいHRを受けた時点で接続状態にする。

SensorsではOAuth Device Flowで認証する。指定client_id `5131ba31-97fa-4587-b9d8-e155640f343e` とscope `data:heart_rate:read` をPOSTし、認証リンクを表示する。ユーザー承認後にtoken endpointへdevice_code grantでアクセスし、心拍ストリームへ自動接続する。interval/slow_down/期限切れ/拒否/中止を処理し、client_secretを埋め込まない。headlessではPULSOID_ACCESS_TOKENを維持する。永続設定に含めず、第三者ライブラリのWebSocketトレースも抑止する。

## モードと自動記録

- HRなし: unavailable、判定不能。
- HRのみ: hr_only、擬似睡眠を出力。RR/HRV UIなし。Sleep操作は無効。
- HR＋最近のRR: hr_rr。HRV・品質・Baseline条件を満たした場合のみ本計測として推定。
- モード変更時は旧モードの睡眠状態と継続タイマーを引き継がない。
- 初期の睡眠確定時間は300秒、設定範囲60〜1800秒。覚醒確認は120秒。
- 自動セッションはhr_rr、Tracking=true、sleep_like=trueの場合だけ作成。
- 確定前の集計は36区間の上限付きキャッシュに保持し、確定後にPersonalDataへ保存。Rawは常時PersonalDataへ保存しているため再解析可能。
- Sessionは判定不能・RR喪失・切断でも閉じる。欠測を睡眠時間へ加算しない。

## DBとメモリ

PersonalDataは永続Raw・Session・設定。Cacheは再生成可能な1分集計で35日保持。DBライターを分け、Cacheの削除で実測データを失わない構成。SQLiteは削除済みページを再利用するためCacheファイルサイズが直ちに縮むとは限らない。

表示キャッシュはGUIの最大時間幅に合わせ1805点。RR Bufferは従来どおり6000通知/20分の上限。Sessionの平均HRはsum/count、RMSSD中央値は2001ビンのヒストグラム（1ms丸め）とし、長期のベクトル増加を除去。GUIは全期間の特徴量をロードせず選択したSessionのみ取得する。長い1セッションのグラフは最大20160区間に限定するが、DB上の元記録は削除しない。

旧DBはSQLite Backup APIでWAL込みでコピーし、旧pendingファイルも新しい場所で再投入する。移行元は残す。

## 外部仕様

VRCOSC/SleepにAsleep / Estimated / MeasurementAvailableを追加。既存Enabled / Tracking / Probabilityは維持。RESTの/api/v1/sleep/statusでも同じ区別を提供。既存SnapshotとUnix IPCにmode / rr_available / source_timestampを追加する。

## 未検証・変更していない範囲

Pulsoid実接続はユーザーのトークンで確認が必要。Windows AppData分岐はコード/パステスト済みだがWindows上の実行は未検証。医療的な睡眠判定精度、実機の一晩計測、Shared Memory、Windows Named Pipeは今回の完成範囲外。

参照: https://docs.pulsoid.net/read-heart-rate/read-heart-rate-via-websocket

## OSCQueryとロゴ（追加）

vrchat_osc 2.2でOSC/OSCQueryのmDNSサービスを検出。同じVRChat-Client名でペアにし、OSCQuery HTTPルートが応答した宛先だけへ送信する。最大8クライアント、5秒周期の再検証、2秒タイムアウト。HTTP問い合わせは別タスクで行う。UDP送信成功だけをVRChat検出と解釈しない。自動検出をOFFにすると既存の手動UDP送信を使用する。

ユーザー指定のnemuribeat-moon-pulse-v0.4をロゴ・eguiアイコン・Windows EXEアイコン・macOS ICNSとして同梱。元画像の図案は変更しない。
