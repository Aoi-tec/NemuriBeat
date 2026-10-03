# 2026-10-02 追加要件と実装

製品名はNemuriBeat。既存テーマ・日本語フォント・チャート構成を維持し、入力モードに応じてUIを切り替える。

## 入力

HypeRateはsensor-hyperate crateのSensorAdapter。APIキーとIDでWebSocketへ接続し、チャンネル参加後にHRのみのMeasurementをCoreへ渡す。64KiB上限、15秒keepalive、再接続1〜30秒、401/403・参加拒否は停止。RRと測定元の時刻は生成しない。source_timestampはnull。同じBPMの新着通知を捨てない。

SensorsにIDとマスク付きAPIキー入力を用意。IDだけ設定に保存できる。APIキーはメモリのみ。headlessではHYPERATE_API_KEY / HYPERATE_IDから読む。旧Pulsoid OAuth実装・client_id・接続コマンドは置換して削除した。

## モードと自動記録

- HRなし: unavailable、判定不能。
- HRのみ: hr_only、擬似睡眠を出力。RR/HRV UIなし。Sleep操作は無効。
- HR＋最近のRR: hr_rr。品質・公式SleepECG GRU・HMD条件を満たした場合のみ本計測として推定。
- モード変更時は旧モードの睡眠状態と継続タイマーを引き継がない。
- 初期の睡眠確定時間は300秒、設定範囲60〜1800秒。覚醒確認は120秒。
- 自動セッションはhr_rr、Tracking=true、sleep_like=trueの場合だけ作成。
- 確定前の集計は36区間の上限付きキャッシュに保持し、確定後にPersonalDataへ保存。Rawは常時PersonalDataへ保存しているため再解析可能。
- Sessionは判定不能・RR喪失・切断でも閉じる。欠測を睡眠時間へ加算しない。

## DBとメモリ

PersonalDataは永続Raw・Session・設定。Cacheは再生成可能な1分集計で35日保持。DBライターを分け、Cacheの削除で実測データを失わない構成。SQLiteは削除済みページを再利用するためCacheファイルサイズが直ちに縮むとは限らない。

表示キャッシュはGUIの最大時間幅に合わせ1805点。RR Bufferは従来どおり6000通知/20分の上限。Sessionの平均HRはsum/count、RMSSD中央値は2001ビンのヒストグラム（1ms丸め）とし、長期のベクトル増加を除去。GUIは全期間の特徴量をロードせず選択した睡眠日の区間を取得する。長い1セッションのグラフは最大20160区間に限定するが、DB上の元記録は削除しない。

旧DBはSQLite Backup APIでWAL込みでコピーし、旧pendingファイルも新しい場所で再投入する。移行元は残す。

## 外部仕様

VRCOSC/SleepにAsleep / Estimated / MeasurementAvailableを追加。既存Enabled / Tracking / Probabilityは維持。RESTの/api/v1/sleep/statusでも同じ区別を提供。既存SnapshotとUnix IPCにmode / rr_available / source_timestampを追加する。

## 未検証・変更していない範囲

HypeRate実接続はユーザーのIDとAPIキーで確認が必要。Windows AppData分岐はコード/パステスト済みだがWindows上の実行は未検証。医療的な睡眠判定精度、実機の一晩計測、Shared Memory、Windows Named Pipeは今回の完成範囲外。

参照: https://github.com/alexholzreiter/HypeRate-Websocket-API/blob/main/docs/WEBSOCKET_API.md

## OSCQueryとロゴ（追加）

vrchat_osc 2.2でOSC/OSCQueryのmDNSサービスを検出。同じVRChat-Client名でペアにし、OSCQuery HTTPルートが応答した宛先だけへ送信する。最大8クライアント、5秒周期の再検証、2秒タイムアウト。HTTP問い合わせは別タスクで行う。UDP送信成功だけをVRChat検出と解釈しない。自動検出をOFFにすると既存の手動UDP送信を使用する。

ユーザー指定のnemuribeat-moon-pulse-v0.4をロゴ・eguiアイコン・Windows EXEアイコン・macOS ICNSとして同梱。元画像の図案は変更しない。

## 常駐とWebオーバーレイ（2026-10-03）

Windows/macOSでtray-iconをメインイベントループ初回update時に作成。CloseRequestedをCancelClose＋Visible(false)へ変換し、表示復帰と終了をトレイメニューで扱う。終了は従来のbackend停止・DB flush経路を通す。トレイが利用できない場合は閉じる操作を妨げず、画面を隠して取り残さない。

共通URL `/overlay/heart-rate` をXSOverlay/OVR ToolkitのBuilt-in Web Browser向けに配信。最小JSON `/api/v1/overlay/status` はno-store。Web配信はoverlay_enabled（初期true）、全RESTはapi_enabled（初期false）で分離。どちらかが有効ならapi_portでlocalhost待ち受けを行う。

背景にはBPMから生成した心電図風の一本の線をCanvasで描画。60000/BPM ms周期の擬似波形を左へ流し、最大30fpsで描画する。100Hz・6秒分の固定Float32Array（600点、約2.4KiB）だけを保持し、BPM変更時も位相を維持する。受信停止・通信失敗で履歴を消去して水平線へ戻す。非表示時およびprefers-reduced-motion時は描画ループを停止する。実心電図や個々の実拍動時刻は表現しない。

最新版ユーザー指示に合わせ、両方の表示をHTTP/HTML方式へ統一。C#プラグインは使用しない。ブラウザの表示コードはローカル同梱し、外部CDN・APIキーのブラウザ配布は行わない。

## 睡眠モデル・HMD・日次集約（2026-10-03）

最新仕様は `SLEEP_GRU_OPENVR.md` を参照。RR実運用は公式ws-gru-mesa（270秒lookforward、約4〜5分遅延）、旧ルールはデモ用。HMDの速度差分加速度・近接装着情報を使い、ゼロ平均・未装着・無効追跡を棄却。Sleep / Historyは保存済みsleep_dayで同日のSessionを集約し、欠測を埋めない。公式GRU以外の重みを持たないというユーザー回答に従い、CNNは未実装の残件とする。未完了一覧はルートTask.MD。

## 2026-10-03 OyasumiVR追加

NemuriBeatからOyasumiVRのSleepModeへ一方向に連携。OSCQueryで自動検出し、現在値が異なる場合だけBoolを送信する。判定不能・無効化・終了時は状態を保持。HRのみの擬似判定は別途許可が必要。手順と検証の範囲は `OYASUMIVR.md` を参照。
