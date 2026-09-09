# 現在地凍結(location freeze)の検出

テスト乗車のテレメトリから「アプリは動いているのに現在地が進まなくなった」区間を抽出し、
路線・区間・機種・ビルド別に集計するための GraphQL Query 3 本(`locationFreezes` /
`locationFreezeSessions` / `locationFreezeSummary`)について説明します。

関連 Issue: [TrainLCD/THQ#30](https://github.com/TrainLCD/THQ/issues/30)

## シグネチャ(何を「凍結」とみなすか)

MobileApp は乗車中、最大 1 回/秒の頻度で `sendLocation` を送ります。したがって位置ログが
数分単位で途切れていれば、その間クライアントは位置を更新できていません。ただし途切れ自体は
異常とは限らないため、次の 3 条件を **すべて** 満たしたものだけを凍結候補として扱います。

1. **位置ログの欠落**: 同一 `session_id` の連続する 2 行の間隔が `gapThresholdMs`(既定 60000 ms)を超える。
2. **欠落直前の速度が高い**: 欠落直前の行の OS 由来 `speed` が `speedThresholdKmh`(既定 30 km/h)を超える。
3. **その間もアプリは生きていた**: 欠落期間中に同一 `session_id` の `log_events` /
   `interaction_events` が 1 件以上ある(`requireAppAlive: true`、既定)。

各条件が切り分けているものは次のとおりです。

- 条件 2 は **駅停車** を除外します。停車中に位置更新が止まっても、それは正常です。
- 条件 3 は **アプリ・端末の死亡** を除外します。強制終了・電源断・バックグラウンド停止で
  ログ全体が止まったのであれば、位置ログだけの問題ではありません。
- 残るのは「アプリのイベントは流れ続けているのに位置ログだけが止まった」ケース、つまり
  画面上の現在地が高速移動中に凍結した状態です。

## 3 つの Query の使い分け

いずれも観測用トークン(`THQ_OBSERVER_AUTH_TOKEN`)と DB 接続が必要で、同じ
`LocationFreezeFilter` を取ります。`from` / `to` は必須で、差は最大 90 日です。

| Query | 粒度 | 並び順 | 主な用途 |
|---|---|---|---|
| `locationFreezes` | 欠落 1 件ごと | `gapStart` 降順 | 個別事象の再現条件を調べる |
| `locationFreezeSessions` | セッションごと | `startedAt` 降順 | 1 回の乗車が何回凍結したかを見る |
| `locationFreezeSummary` | 路線・区間・機種・ビルドごと | `freezeCount` 降順、`sessionCount` 降順 | ビルド間・区間別の比較 |

`locationFreezeSessions` と `locationFreezeSummary` は **凍結が 0 件のセッション / グループも返します**。
「結果に出てこない」と「凍結が無かった」を区別できないと、ビルド間比較になりません。

### 例: 同一区間を 2 つのビルドで走った結果を並べる

```graphql
query {
  locationFreezeSummary(
    filter: {
      from: "2026-07-01T00:00:00Z"
      to: "2026-07-08T00:00:00Z"
      segmentId: "11302:1130201:1130202"
    }
  ) {
    appVersion
    platform
    device
    sessionCount
    locationCount
    freezeSessionCount
    freezeCount
    maxGapMs
    totalGapMs
  }
}
```

返る行は例えば次のようになります。

```text
appVersion     device      sessionCount  freezeSessionCount  freezeCount  maxGapMs
10.4.1(100)    Pixel 8     1             1                   1            300000
10.4.2(101)    Pixel 8     1             0                   0            null
```

同じ区間・同じ端末で `freezeSessionCount` が 1 → 0 になっているので、その区間については
10.4.2(101) で解消したと読めます。逆に `freezeCount` が増えていれば退行です。

個別事象の詳細を見るときは `locationFreezes` を使います。

```graphql
query {
  locationFreezes(
    filter: {
      from: "2026-07-01T00:00:00Z"
      to: "2026-07-08T00:00:00Z"
      appVersion: "10.4.1(100)"
    }
  ) {
    sessionId
    segmentId
    gapStart
    gapEnd
    gapMs
    speedBeforeGap
    coordsBeforeGap { latitude longitude accuracy speed }
    coordsAfterGap { latitude longitude accuracy speed }
    jumpDistanceMeters
    aliveEventCount
  }
}
```

`jumpDistanceMeters` は `coordsBeforeGap` と `coordsAfterGap` の大円距離(m)で、
凍結中に表示位置が実位置からどれだけ離れたかの目安です。`aliveEventCount` は欠落期間中に
届いた `log_events` + `interaction_events` の件数で、条件 3 の根拠にあたります。

## しきい値の考え方

- **`gapThresholdMs`**: 既定の 60000 ms は、送信間隔(最大 1 回/秒)の 60 倍にあたります。
  地下鉄やトンネル区間では測位が正常に途切れるため、そのままでは正常な欠落を拾います。
  対象を路線で絞る(`lineId` / `segmentId`)か、しきい値を大きめに取ってください。
  THQ は路線種別(地上/地下)を持っていないため、この判断は自動化できません。
- **`speedThresholdKmh`**: 既定の 30 km/h は、駅停車・徐行と走行中を分けるための値です。
  在来線の低速区間を見るときは下げ、新幹線の高速域だけを見るときは上げます。
  なお `speed` は OS 由来の値で、欠落直前の 1 行しか見ていません。
- **`requireAppAlive`**: `false` にすると条件 3 を外し、アプリが落ちた可能性のある欠落も
  含めて返します。「凍結ではなくクラッシュではないか」を確かめるとき、true と false の
  件数差を見るのが手軽です。
- **`limit`**: 既定 100、上限 2000 に丸められます。

## 検出できない欠落

仕組み上、次の 2 つは検出できません。集計を読むときの前提にしてください。

1. **検索窓の末尾にかかる欠落**: 欠落の長さは「次に届いた行」との差で測るため、`[from, to)`
   の窓の中に次の行が入っていない欠落は出てきません。窓を広げるか、後ろにずらしてください。
2. **セッションが戻ってこない欠落**: 電源断やアプリ終了で位置ログが二度と来なかった場合、
   欠落を閉じる行が存在しないため検出されません。これは条件 3 で除外したいケースとも重なります。

また、`session_id` が NULL の行(旧クライアント)は対象外です。

## 過去データの `appVersion` 補完

`location_logs` の `app_version` / `platform` / `channel` は THQ#30 で追加した列です。
それ以前の行と、まだ送っていないクライアントの行では NULL になります。

このため 3 つの Query は、位置ログの列が NULL のときに **同一 `session_id` の
`log_events` / `interaction_events` からビルド情報を補完** します(それぞれの `MIN` を取ります)。
`appVersion` / `platform` / `channel` フィルタも、この補完後の値に対して適用されます。
どちらにも情報が無ければ `null` のままです。

補完はセッション単位なので、1 セッションの途中でアプリを更新するようなケースは表現できません。
実運用では乗車中にビルドが変わることはないため、この単純化を採用しています。

## 実装メモ

- SQL は `src/freeze.rs` の共通 CTE 1 本を 3 つの Query で共有し、末尾の `SELECT` だけを差し替えています。
- `LEAD()` による前後行の対応付けは、**路線・区間・機種などで絞り込む前に**、セッション内の
  全行に対して計算します。先に区間で絞ると区間境界や路線切替をまたぐ隣接行が消え、
  実際には存在しない欠落が生まれるためです。絞り込みは「欠落直前の行」に対して後段で適用します。
- Postgres 統合テストは `THQ_TEST_DATABASE_URL` が設定されているときだけ走ります
  (未設定ならスキップ)。詳細は [README](../README.md) の Testing 節を参照してください。
