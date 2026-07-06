# React.js + TanStack Query で thq-server に接続する

このドキュメントでは、React アプリケーションから [TanStack Query](https://tanstack.com/query/latest)(旧 React Query)を使って thq-server の GraphQL API に接続する方法を説明します。

## 前提

thq-server の GraphQL API はエンドポイント `POST /graphql` で公開されています(Playground: `GET /graphql`)。

| 操作 | 種別 | 認証 |
|---|---|---|
| `sendLogEvent` | Mutation | イベント用または遠隔測定用トークン |
| `sendInteractionEvent` | Mutation | イベント用または遠隔測定用トークン |
| `sendLocation` | Mutation | 遠隔測定用トークンのみ |
| `accuracyByLine` | Query | 不要 |

Mutation の認証は `Authorization: Bearer <token>` ヘッダで行います。

| トークン | できること |
|---|---|
| イベント用(`THQ_EVENTS_AUTH_TOKEN`) | `sendLogEvent` + `sendInteractionEvent` |
| 遠隔測定用(`THQ_TELEMETRY_AUTH_TOKEN`) | `sendLogEvent` + `sendInteractionEvent` + `sendLocation` |

> **セキュリティ上の注意**: ブラウザ向けにビルドした JavaScript に埋め込んだトークンは、利用者全員から見えます。イベント用・遠隔測定用トークンを Web フロントエンドに直接埋め込むのは避け、ネイティブアプリや自前のバックエンド(BFF)経由で扱ってください。認証不要な `accuracyByLine` の表示だけであればトークンは一切不要です。

> **CORS の注意**: 現状の thq-server は CORS ヘッダを返しません。ブラウザから `POST /graphql` を叩く場合は、フロントエンドを同一オリジンで配信するか、リバースプロキシ(nginx 等)を挟んでください。

## セットアップ

```bash
npm install @tanstack/react-query
```

アプリのルートに `QueryClientProvider` を設定します。

```tsx
// main.tsx
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";

const queryClient = new QueryClient();

export function Root() {
  return (
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  );
}
```

## GraphQL クライアント

専用ライブラリは不要で、`fetch` の薄いラッパーで十分です。GraphQL はエラーでも HTTP 200 を返すため、`errors` 配列の確認が必須です。

```ts
// lib/graphql.ts
const GRAPHQL_ENDPOINT = import.meta.env.VITE_THQ_GRAPHQL_URL ?? "/graphql";

export async function gqlRequest<TData, TVariables = Record<string, unknown>>(
  query: string,
  variables?: TVariables,
  token?: string,
): Promise<TData> {
  const res = await fetch(GRAPHQL_ENDPOINT, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify({ query, variables }),
  });

  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }

  const json = await res.json();
  if (json.errors?.length) {
    // 認証エラーは "unauthorized: ..." というメッセージで返る
    throw new Error(json.errors.map((e: { message: string }) => e.message).join("; "));
  }
  return json.data as TData;
}
```

## Query: 回線ごとの精度レポート(`accuracyByLine`)

GraphQL Query は認証不要です。`useQuery` でそのまま取得できます。

```ts
// hooks/useAccuracyByLine.ts
import { useQuery } from "@tanstack/react-query";
import { gqlRequest } from "../lib/graphql";

const ACCURACY_BY_LINE = /* GraphQL */ `
  query AccuracyByLine(
    $lineId: ID!
    $from: DateTime!
    $to: DateTime!
    $bucketSize: TimeBucketSize!
    $limit: Int
  ) {
    accuracyByLine(lineId: $lineId, from: $from, to: $to, bucketSize: $bucketSize, limit: $limit) {
      lineId
      buckets {
        bucketStart
        bucketEnd
        avgAccuracy
        p90Accuracy
        sampleCount
        avgSpeed
        maxSpeed
      }
    }
  }
`;

export interface AccuracyBucket {
  bucketStart: string;
  bucketEnd: string;
  avgAccuracy: number;
  p90Accuracy: number;
  sampleCount: number;
  avgSpeed: number | null;
  maxSpeed: number | null;
}

interface AccuracyByLineData {
  accuracyByLine: { lineId: string; buckets: AccuracyBucket[] };
}

export function useAccuracyByLine(params: {
  lineId: string;
  from: string; // ISO 8601 (例: "2026-07-01T00:00:00Z")
  to: string;
  bucketSize: "minute" | "hour" | "day";
  limit?: number;
}) {
  return useQuery({
    queryKey: ["accuracyByLine", params],
    queryFn: () => gqlRequest<AccuracyByLineData>(ACCURACY_BY_LINE, params),
    staleTime: 60_000, // 集計データなので 1 分程度はキャッシュを新鮮とみなす
  });
}
```

使用例:

```tsx
function AccuracyChart() {
  const { data, isPending, error } = useAccuracyByLine({
    lineId: "11302",
    from: "2026-07-01T00:00:00Z",
    to: "2026-07-06T00:00:00Z",
    bucketSize: "hour",
  });

  if (isPending) return <p>読み込み中…</p>;
  if (error) return <p>エラー: {error.message}</p>;

  return (
    <ul>
      {data.accuracyByLine.buckets.map((b) => (
        <li key={b.bucketStart}>
          {b.bucketStart}: 平均精度 {b.avgAccuracy.toFixed(1)} m({b.sampleCount} 件)
        </li>
      ))}
    </ul>
  );
}
```

バケットサイズごとの最大期間(minute ≤ 7 日、hour ≤ 90 日、day ≤ 365 日)を超えるとエラーになる点に注意してください。

## Mutation: ログイベント送信(`sendLogEvent`)

イベント用または遠隔測定用トークンが必要です。

```ts
// hooks/useSendLogEvent.ts
import { useMutation } from "@tanstack/react-query";
import { gqlRequest } from "../lib/graphql";

const SEND_LOG_EVENT = /* GraphQL */ `
  mutation SendLogEvent($input: LogEventInput!) {
    sendLogEvent(input: $input) {
      sessionId
    }
  }
`;

export interface LogEventInput {
  sessionId: string; // 必須。クライアント側で生成した一意な文字列(後述)
  device?: string; // 匿名性確保のため省略可。省略時は null として配信・保存される
  appVersion: string; // 必須。アプリのバージョン文字列(空はサーバーが拒否)
  platform: "ios" | "android" | "macos" | "unknown"; // 必須
  channel: "production" | "canary"; // 必須
  timestamp: number; // Unix ミリ秒 (Date.now())
  type: "system" | "app" | "client";
  level: "debug" | "info" | "warn" | "error";
  message: string; // 空文字・空白のみはサーバーが拒否
}

export function useSendLogEvent(token: string) {
  return useMutation({
    mutationFn: (input: LogEventInput) =>
      gqlRequest<{ sendLogEvent: { sessionId: string } }>(SEND_LOG_EVENT, { input }, token),
  });
}
```

使用例:

```tsx
const sendLog = useSendLogEvent(eventsToken);

sendLog.mutate({
  sessionId,
  device: "device-001",
  appVersion: "1.2.3",
  platform: "ios",
  channel: "production",
  timestamp: Date.now(),
  type: "app",
  level: "info",
  message: "GPS signal acquired",
});
```

端末を特定されたくない場合は `device` を省略して匿名で送信できます:

```ts
sendLog.mutate({
  sessionId,
  appVersion: "1.2.3",
  platform: "ios",
  channel: "production",
  timestamp: Date.now(),
  type: "app",
  level: "info",
  message: "started",
});
```

> `timestamp` はスキーマ上 `Int!` と表示されますが、サーバー内部は 64bit 整数のため `Date.now()` の値(約 1.7 兆)をそのまま渡して問題ありません。

### sessionId の生成

`sessionId` はクライアント側で生成する一意な文字列で、両 Mutation で必須です。セッション(アプリ起動)ごとに 1 回生成して、そのセッション中のすべての送信で使い回す想定です。

```ts
// アプリ起動時に 1 回だけ生成する
const sessionId = crypto.randomUUID();
```

なお、イベント自体の ID はサーバー側で常に UUID が採番されます。クライアントから ID を指定することはできないため、送信リクエストの再送はそれぞれ別イベントとして記録される点に注意してください。

## Mutation: インタラクションイベント送信(`sendInteractionEvent`)

ユーザー主導のインタラクション(行動)を任意のイベント名で記録します。`sendLogEvent` が `console.*` 相当の出力を送る想定なのに対し、こちらは「何をしたか」をイベント名で記録する用途です。認証は `sendLogEvent` と同じで、観測用以外のトークン(イベント用または遠隔測定用)で送信できます。

イベント名は任意の文字列です。例:

| eventName | 意味 |
|---|---|
| `app_launch` | アプリ起動 |
| `tab_change` | アプリのタブ移動 |
| `tts_request` | TTS(Text-to-Speech)のリクエスト |
| `tts_success` / `tts_failure` | TTS リクエストの成功・失敗 |
| `feedback_request` | フィードバックの送信リクエスト |
| `feedback_success` / `feedback_failure` | フィードバック送信の成功・失敗 |

```ts
// hooks/useSendInteractionEvent.ts
import { useMutation } from "@tanstack/react-query";
import { gqlRequest } from "../lib/graphql";

const SEND_INTERACTION_EVENT = /* GraphQL */ `
  mutation SendInteractionEvent($input: InteractionEventInput!) {
    sendInteractionEvent(input: $input) {
      sessionId
    }
  }
`;

export interface InteractionEventInput {
  sessionId: string; // 必須。クライアント側で生成した一意な文字列
  device?: string; // 匿名性確保のため省略可
  appVersion: string; // 必須。アプリのバージョン文字列(空はサーバーが拒否)
  platform: "ios" | "android" | "macos" | "unknown"; // 必須
  channel: "production" | "canary"; // 必須
  timestamp: number; // Unix ミリ秒 (Date.now())
  eventName: string; // 任意のイベント名。空文字・空白のみはサーバーが拒否
  properties?: Record<string, string | number | boolean | null>; // 省略可(後述)
}

export function useSendInteractionEvent(token: string) {
  return useMutation({
    mutationFn: (input: InteractionEventInput) =>
      gqlRequest<{ sendInteractionEvent: { sessionId: string } }>(
        SEND_INTERACTION_EVENT,
        { input },
        token,
      ),
  });
}
```

使用例:

`properties` はイベントに付随する属性を格納するフラットなオブジェクトです。値に使えるのは文字列・数値・真偽値・null のみで、ネストしたオブジェクトや配列はサーバーが拒否します(GraphQL 上はカスタムスカラー `Properties`)。

```tsx
const sendInteraction = useSendInteractionEvent(eventsToken);

// アプリ全体で共通の属性はラップしておくと便利
const track = (
  eventName: string,
  properties?: Record<string, string | number | boolean | null>,
) =>
  sendInteraction.mutate({
    sessionId,
    appVersion: "1.2.3",
    platform: "ios",
    channel: "production",
    timestamp: Date.now(),
    eventName,
    properties,
  });

// アプリ起動時
track("app_launch");

// タブ移動(付随情報は properties で)
track("tab_change", { tab: "map", index: 2 });

// TTS リクエストの結果
try {
  await requestTts(text);
  track("tts_success");
} catch (err) {
  track("tts_failure", { reason: String(err) });
}
```

## Mutation: 位置情報送信(`sendLocation`)

**遠隔測定用トークンのみ**が実行できます。イベント用トークンでは `unauthorized: a valid telemetry bearer token is required` エラーになります。また、`sendLogEvent` と異なり **`device` は必須**です(位置情報は端末と紐付いていることが前提のため、匿名では送信できません)。

```ts
// hooks/useSendLocation.ts
import { useMutation } from "@tanstack/react-query";
import { gqlRequest } from "../lib/graphql";

const SEND_LOCATION = /* GraphQL */ `
  mutation SendLocation($input: LocationEventInput!) {
    sendLocation(input: $input) {
      sessionId
      warning
    }
  }
`;

export interface LocationEventInput {
  sessionId: string; // 必須。クライアント側で生成した一意な文字列
  device: string; // 必須(sendLogEvent と異なり省略不可)
  state: "arrived" | "approaching" | "passing" | "moving";
  stationId?: number; // arrived / passing のときのみ有効。moving / approaching では無視される
  lineId: number;
  coords: {
    latitude: number; // -90〜90
    longitude: number; // -180〜180
    accuracy?: number; // メートル。負値はエラー
    speed?: number; // km/h。負値は「不明」として NULL 扱い
  };
  timestamp: number; // Unix ミリ秒
  batteryLevel?: number; // 0.0〜1.0
  batteryState?: "unknown" | "unplugged" | "charging" | "full";
}

interface SendLocationData {
  sendLocation: { sessionId: string; warning: string | null };
}

export function useSendLocation(token: string) {
  return useMutation({
    mutationFn: (input: LocationEventInput) =>
      gqlRequest<SendLocationData>(SEND_LOCATION, { input }, token),
    onSuccess: (data) => {
      if (data.sendLocation.warning) {
        // 例: "reported accuracy 150.0m exceeds threshold 100m"
        console.warn("thq-server warning:", data.sendLocation.warning);
      }
    },
  });
}
```

Geolocation API と組み合わせる例:

```tsx
const sendLocation = useSendLocation(telemetryToken);

useEffect(() => {
  const watchId = navigator.geolocation.watchPosition((pos) => {
    sendLocation.mutate({
      sessionId,
      device: "device-001",
      state: "moving",
      lineId: 11302,
      coords: {
        latitude: pos.coords.latitude,
        longitude: pos.coords.longitude,
        accuracy: pos.coords.accuracy,
        speed: pos.coords.speed != null ? pos.coords.speed * 3.6 : undefined, // m/s → km/h
      },
      timestamp: pos.timestamp,
    });
  });
  return () => navigator.geolocation.clearWatch(watchId);
}, []);
```

## 接続先の設定(環境変数)

Vite の場合:

```bash
# .env.local (フロントエンド側リポジトリ)
VITE_THQ_GRAPHQL_URL=https://thq.example.com/graphql
```

繰り返しになりますが、`VITE_` プレフィックスの環境変数はビルド成果物に埋め込まれ公開されます。イベント用・遠隔測定用トークンはビルドに埋め込まず、BFF 等のサーバーサイドで保持してください。
