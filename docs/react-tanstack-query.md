# React.js + TanStack Query で thq-server に接続する

このドキュメントでは、React アプリケーションから [TanStack Query](https://tanstack.com/query/latest)(旧 React Query)を使って thq-server の GraphQL API に接続する方法を説明します。

## 前提

thq-server の GraphQL API はエンドポイント `POST /graphql` で公開されています(Playground: `GET /graphql`)。

| 操作 | 種別 | 認証 |
|---|---|---|
| `sendLogEvent` | Mutation | イベント用または遠隔測定用トークン |
| `sendLocation` | Mutation | 遠隔測定用トークンのみ |
| `accuracyByLine` | Query | 不要 |

Mutation の認証は `Authorization: Bearer <token>` ヘッダで行います。

| トークン | できること |
|---|---|
| イベント用(`THQ_EVENTS_AUTH_TOKEN`) | `sendLogEvent` のみ |
| 遠隔測定用(`THQ_TELEMETRY_AUTH_TOKEN`) | `sendLogEvent` + `sendLocation` |

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
  bucketSize: "MINUTE" | "HOUR" | "DAY";
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
    bucketSize: "HOUR",
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

バケットサイズごとの最大期間(MINUTE ≤ 7 日、HOUR ≤ 90 日、DAY ≤ 365 日)を超えるとエラーになる点に注意してください。

## Mutation: ログイベント送信(`sendLogEvent`)

イベント用または遠隔測定用トークンが必要です。

```ts
// hooks/useSendLogEvent.ts
import { useMutation } from "@tanstack/react-query";
import { gqlRequest } from "../lib/graphql";

const SEND_LOG_EVENT = /* GraphQL */ `
  mutation SendLogEvent($input: LogEventInput!) {
    sendLogEvent(input: $input) {
      id
    }
  }
`;

export interface LogEventInput {
  id?: string; // 省略時はサーバーが UUID を採番
  device: string;
  timestamp: number; // Unix ミリ秒 (Date.now())
  type: "SYSTEM" | "APP" | "CLIENT";
  level: "DEBUG" | "INFO" | "WARN" | "ERROR";
  message: string; // 空文字・空白のみはサーバーが拒否
}

export function useSendLogEvent(token: string) {
  return useMutation({
    mutationFn: (input: LogEventInput) =>
      gqlRequest<{ sendLogEvent: { id: string } }>(SEND_LOG_EVENT, { input }, token),
  });
}
```

使用例:

```tsx
const sendLog = useSendLogEvent(eventsToken);

sendLog.mutate({
  device: "device-001",
  timestamp: Date.now(),
  type: "APP",
  level: "INFO",
  message: "GPS signal acquired",
});
```

> `timestamp` はスキーマ上 `Int!` と表示されますが、サーバー内部は 64bit 整数のため `Date.now()` の値(約 1.7 兆)をそのまま渡して問題ありません。

### 再送とべき等性

`id` にクライアント側で生成した UUID を渡しておくと、同じ `id` の再送はサーバー側で無視(`ON CONFLICT DO NOTHING`)されるため、ネットワークエラー時のリトライを安全に行えます。

```ts
sendLog.mutate({
  id: crypto.randomUUID(),
  device: "device-001",
  timestamp: Date.now(),
  type: "CLIENT",
  level: "ERROR",
  message: "Connection lost",
});
```

## Mutation: 位置情報送信(`sendLocation`)

**遠隔測定用トークンのみ**が実行できます。イベント用トークンでは `unauthorized: a valid telemetry bearer token is required` エラーになります。

```ts
// hooks/useSendLocation.ts
import { useMutation } from "@tanstack/react-query";
import { gqlRequest } from "../lib/graphql";

const SEND_LOCATION = /* GraphQL */ `
  mutation SendLocation($input: LocationEventInput!) {
    sendLocation(input: $input) {
      id
      warning
    }
  }
`;

export interface LocationEventInput {
  id?: string;
  device: string;
  state: "ARRIVED" | "APPROACHING" | "PASSING" | "MOVING";
  stationId?: number; // ARRIVED / PASSING のときのみ有効。MOVING / APPROACHING では無視される
  lineId: number;
  coords: {
    latitude: number; // -90〜90
    longitude: number; // -180〜180
    accuracy?: number; // メートル。負値はエラー
    speed?: number; // km/h。負値は「不明」として NULL 扱い
  };
  timestamp: number; // Unix ミリ秒
  batteryLevel?: number; // 0.0〜1.0
  batteryState?: "UNKNOWN" | "UNPLUGGED" | "CHARGING" | "FULL";
}

interface SendLocationData {
  sendLocation: { id: string; warning: string | null };
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
      device: "device-001",
      state: "MOVING",
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
