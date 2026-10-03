# SleepECG公式GRU・HMD条件・日次集約

2026-10-03。ユーザーの最新指定「公式GRUを使用、ほかの重みはない」に従う。CNNの追加は保留し、精度が上がったとは認定しない。

## 推定経路

```text
BLEの実RR → 20分/6000通知の有界窓 → 専用Pythonワーカー
  → SleepECG公式前処理 → ws-gru-mesa → 完了30秒epochの睡眠確率
OpenVRのPose速度 → 10Hz差分加速度・30秒の平均／近接装着状態
  → 品質・鮮度・HMD条件 → EMA／継続確認 → Core・OSC・保存
```

公式GRUの配布重みを `sleepecg.load_classifier("ws-gru-mesa", "SleepECG")` で読み、`sleepecg.stage(..., return_mode="prob")` を呼ぶ。分類順序はUNDEFINED / SLEEP / WAKE。UNDEFINED優勢、不正値、推論失敗は判定不能。

配布モデルのlookbackは240秒、lookforwardは270秒。入力を正確に合わせ、直近の完了epochのうち先読み270秒を確保できたものを採用する。蓄積は少なくとも約540秒、出力は約4〜5分遅れる。前処理を勝手に現在時刻だけへ変えない。APIの`model.evaluated_at`に推定対象epochの終端、`received_at`にモデル結果を受け取った時刻を返す。セッションの継続確認・保存開始はアプリが判定を受けた時刻を基準とし、遅延分の未判定時間を自動で睡眠としてさかのぼり加算しない。

学習は全夜の文脈を使うGRU。20分の移動窓への適用、既定の欠けた年齢/性別入力、BLE通知からのRR時系列の扱いは実睡眠で未評価。現段階の値は推定値であり、VR環境での精度向上を検証した値ではない。

HRからRRを生成しない。RRの間隔を累積して拍動列を構成し、短い／長い実RRは時間を削らずSleepECG公式のmin/max RRI前処理へ渡す。接触不良、非正・非有限RR、5秒超の欠測は連続区間をリセット。HypeRateは公式APIにRRがないためHRのみの擬似判定に留める。

CoreのRR品質0.8以上・有効RR30個、モデル受信後90秒以内、推定対象時刻が現在より360秒以内であることを確認。入力世代を変えた際は旧モデル結果を破棄する。重い処理は別プロセスで行い、要求はlatest-only、推論timeout30秒、初期ロード90秒、再試行30秒。モデル停止時に旧RRヒューリスティックへ黙って戻らない。旧ルールはデモだけで使う。

## OpenVR

現在の睡眠グラフの1分足は結果受信時刻に対応する。推定対象epochの履歴保存・グラフ整列はTask.MDの残件であり、グラフの時刻を遅延のない実入眠時刻としては扱わない。

`VRApplication_Background`で読み取り専用接続し、`GetDeviceToAbsoluteTrackingPose(..., prediction=0)`を10Hzで呼ぶ。通常のPoseには生のIMU加速度がないため、`vVelocity`差分 / 実際の経過秒から追跡空間の線形加速度を算出する。重力を含む生の加速度ではない。

30秒の時間加重平均は`|a|`を対象とする。各軸平均の正負相殺を「完全0」と誤解しない。平均が厳密に0なら棄却。丸めて0にしない。有効追跡中の静止条件は平均`|a| <= 0.03m/s²`、角速度`<= 0.04rad/s`、速度`<= 0.02m/s`。閾値は暫定で、HMDごとの校正が残る。

`Prop_ContainsProximitySensor_Bool`がtrueで、HMDの`GetControllerState`が有効に読めた場合は`k_EButton_ProximitySensor`のpressedビットを装着情報に使う。未装着なら棄却。未対応・エラーの場合は`worn=null`を保持し、加速度等から判断する。旧ControllerStateは機種によって取得できず、その場合の最新Input API対応や実機確認は残る。ActivityLevelは移動と近接を混ぜるので装着の代用にはしない。

不正Pose・追跡断・受信後2秒超は棄却。30秒の蓄積前も判定不能。有効な動きを検出したときはRRモデルの睡眠確率を0.1以下に抑え、通常の覚醒ヒステリシスへ渡す。HRのみの擬似判定は静止条件を満たさなければ解除する。

Settingsの「HMDの静止・装着条件」は初期ON。SteamVRを使わない計測ではOFFにしてGRU単独を明示的に選べる。HMDなしを装着・静止と捏造しない。

## 導入

GRUはRRを取得して最初の推論要求が届いてからロードする。HypeRateのHR表示だけではTensorFlowを起動しない。

アプリのHR表示・OSC・Web・デモはPython不要。実RRのGRU推定にはPython 3.12の専用環境が必要。

Windowsの開発チェックアウトで:

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\python.exe -m pip install -r apps/desktop/python/requirements-sleep.txt
```

macOS/Linux:

```sh
python3.12 -m venv .venv
.venv/bin/python -m pip install -r apps/desktop/python/requirements-sleep.txt
```

Settingsの「睡眠モデル用Python実行ファイル」に、その環境のpython.exe / pythonの絶対パスを指定して保存する。設定未指定の初期値は`python`（Windows）/`python3`（それ以外）。環境変数NEMURIBEAT_PYTHONも未保存設定の初期値として使える。自動ランタイム配布・Windowsインストーラーは未完了。

このmacOSで試験済みの環境: `/Users/gomachan/Documents/Codex/2026-10-01/kono/work/nemuri-sleep-venv/bin/python`。このパスは開発環境用で配布先には埋め込まない。

## 日次集約

デフォルトはローカル12:00〜翌12:00を1睡眠日とし、0〜23時へ変更可能。新しいSessionはsleep_dayを保存する。既存Sessionは現在のローカル日付・区切りから導出するが、元の記録を上書きしない。

DBのsession_idと区間は保持。Sleep/Historyはday:YYYY-MM-DDの集約を表示し、すべての区間を同じグラフへ載せる。睡眠時間は区間に記録した睡眠時間の合計で、最初〜最後の差ではない。欠測は睡眠に加算しない。個々の覚醒候補回数を足すが、切断による分割を覚醒と仮定しない。

日次平均HRは区間の睡眠時間で加重した近似値。複数区間のRMSSD中央値は中央値の中央値で代用せず未算出。この統計の厳密な再集計はTask.MDに残す。

RESTを有効にしたときだけ:

- `GET /api/v1/sleep/days`: 睡眠日、集約summary、元segments。
- `GET /api/v1/sleep/days/YYYY-MM-DD`: 上記と同日のminuteグラフ。
- 既存`/sleep/sessions`と個別詳細は元の区間を返して互換を維持。
- `/sleep/status`とSnapshot / Unix IPCにはmodelとhmdを追加。

## 根拠

- [SleepECG公式分類モデル](https://sleepecg.readthedocs.io/en/stable/classification/)
- [SleepECG stage API](https://sleepecg.readthedocs.io/en/stable/api/classification/)
- 配布物sleepecg 0.5.9の`classifiers/ws-gru-mesa.zip/info.yml`と重みを実際に読み取り、前処理と入出力形状を確認。
- [Valve OpenVRヘッダー](https://github.com/ValveSoftware/openvr/blob/master/headers/openvr.h)
- [Valve OpenVR Driver API](https://github.com/ValveSoftware/openvr/blob/master/docs/Driver_API_Documentation.md)
