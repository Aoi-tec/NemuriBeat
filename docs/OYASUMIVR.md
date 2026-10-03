# OyasumiVR連携

更新: 2026-10-03。NemuriBeat → 同じPCのOyasumiVRへ睡眠状態を渡す。

## 設定

1. OyasumiVRを起動し、設定のOSCサーバーを有効にする。
2. NemuriBeatのSettings → OyasumiVRで「睡眠モードを連携」をONにして「適用」。初期値はOFF。
3. OSCQueryポートは `0` のままで自動検出。検出できない場合はOyasumiVRが表示する **OSCQueryのポート** を指定する。VRChatの9000やOSC UDPポートではない。OyasumiVR再起動で手動ポートが変わった場合は再設定する。
4. HypeRateなどHRのみを使う場合は「HRのみの擬似判定も送る」をONにして適用する。初期値OFFなので、初期設定では有効なHR＋RRの睡眠判定だけを連携する。

OyasumiVRの睡眠モードが切り替わると、同アプリで設定してある明るさ・マイク・ステータス等の自動操作が動く。NemuriBeatはその自動操作の設定を変更しない。NemuriBeatを判定元にする場合、OyasumiVR側の独自睡眠検出・時刻・動き・心拍等によるモード切替と競合しないように設定する。NemuriBeatは有効な判定がある間、最大約5秒ごとに自身の判定へ同期するため、OyasumiVR側の手動切替や他の自動切替が戻されることがある。

VRChat OSCのON/OFF、NemuriBeatのREST/Web配信とは独立して使える。Pulsoid認証・HypeRate APIキーをOyasumiVRへ渡す必要はない。

## 通信と判定の扱い

- OSCQueryのmDNSサービス `OyasumiVR._oscjson._tcp.local.` を探す。候補は最大8ポート。
- 同じPCの `http://127.0.0.1:<query-port>/?HOST_INFO` と `/OyasumiVR/SleepMode` を取得する。ホスト名 `OyasumiVR`、UDP、非ゼロOSCポート、読み書き可能なBoolであることを検証する。
- 公式サーバーの `VALUE_TYPE` と標準の `TYPE` の両方に対応。`VALUE[0]` で現在の睡眠モードを読み戻す。
- 状態が異なる場合だけ、取得したUDPポートへ `/OyasumiVR/SleepMode`、OSC Bool `true`（睡眠）／`false`（覚醒）を送る。次のHTTP取得で一致を確認する。UDP送信成功だけでは適用済みと表示しない。
- 睡眠ONには既存の継続確認後の `sleep_like=true` を使う。RRの場合は公式GRU・品質・HMD条件、HRのみの場合は擬似判定の条件をそのまま継承する。GRUの約4〜5分の遅延を解消するものではない。
- 切断・古い入力・Tracking=false・睡眠推定OFF・対象外モードは判定不能／対象外として**送信保留**。OyasumiVR側の現在の状態を保持し、覚醒とみなさない。HMDを外したことによる棄却も自動の覚醒命令にはしない。
- 連携OFF・NemuriBeat終了時も解除命令を送らない。必要ならOyasumiVRで手動解除する。有効な判定が復帰すると同期を再開する。
- OyasumiVR停止・HTTP異常では送信を止める。復帰後はHTTPでポートと状態を再確認する。複数の有効な相手を検出した場合は送信保留し、手動OSCQueryポート指定を使う。
- 通信は別Tokioタスク。mDNSクライアントは初回の自動検出時に一度だけ作成し、設定変更で増やさない。連携OFFではHTTP・OSC送信を停止する。Snapshotと設定は最新値だけのwatch、HTTP応答上限64KiB、HTTPタイムアウト1秒、候補確認上限3秒。計測・DB保存は通信を待たない。
- 接続先は同じPCのIPv4 localhostに限定。広告された外部IPへは送らず、HTTPリダイレクトとプロキシも使わない。

OyasumiVRのSleepModeはBoolだけなので、擬似判定とRR判定の区別はOyasumiVRへ渡らない。NemuriBeatの設定で擬似判定を明示的に許可する。区別が必要な別Consumerには既存の `/api/v1/sleep/status` の `estimated` / `measurement_available` / `mode` を使う。

## 確認した公式実装

- [SleepMode OSCメソッド](https://github.com/Raphiiko/OyasumiVR/blob/99cf20b1e5520503288f09ddd93af2dcc0b61ebb/src-ui/app/services/osc-control/methods/sleep-mode.osc-method.ts): `/OyasumiVR/SleepMode` はReadWrite Bool。`OSC_CONTROL` を理由としてsleep modeを有効／無効にする。
- [OSCサーバー起動](https://github.com/Raphiiko/OyasumiVR/blob/99cf20b1e5520503288f09ddd93af2dcc0b61ebb/src-core/src/osc/commands.rs): 可変UDPポート、OSCQueryの名前は `OyasumiVR`。
- [OSCQueryサーバー](https://github.com/Raphiiko/oyasumivr_oscquery/blob/1bfd8ea4943822ebda2538c7c0d46a484cc5e96d/src/server.rs) と [モデル](https://github.com/Raphiiko/oyasumivr_oscquery/blob/1bfd8ea4943822ebda2538c7c0d46a484cc5e96d/src/models.rs): HOST_INFO、localhost HTTP、BoolのVALUE、VALUE_TYPE、ACCESS。

## 検証と未完了

ローカル模擬相手でOSC Bool、readback、自動検出、HTTP断、UDPポート変更、判定不能／無効／終了時の保持を検証する。Windows上の実OyasumiVR / SteamVR / VRChatでの適用と自動操作の実機確認は未完了。配布済みOyasumiVRの版と上記ソースの差も実機で確認する。手順は `tests/oyasumi_smoke.py target/debug/nemuribeat`、残件は `Task.MD`。
