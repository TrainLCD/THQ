# React.js + TanStack Query で thq-server のデータを WebSocket 観測する

このドキュメントでは、React アプリケーションから thq-server の WebSocket エンドポイントに接続し、登録されたイベント(位置情報・ログ)をリアルタイムに観測する方法を説明します。イベントの送信方法(GraphQL)については [react-tanstack-query.md](./react-tanstack-query.md) を参照してください。

## 前提

| 項目 | 内容 |
|---|---|
| エンドポイント | `ws://<host>:<port>/ws`(TLS 環境では `wss://`) |
| 認証 | 観測用トークン(`THQ_OBSERVER_AUTH_TOKEN`)のみ |
| 方向 | サーバー → クライアントの配信専用(送信は GraphQL 側で行う) |

WebSocket を購読できるのは**観測用トークンだけ**です。イベント用・遠隔測定用トークンでは接続がハンドシェイク時に HTTP 401 で拒否されます。逆に観測用トークンでは GraphQL Mutation を実行できないため、閲覧用クライアントに配るトークンとして安全に分離されています。

> ブラウザ向けビルドに埋め込んだトークンは利用者から見えます。観測用トークンは読み取り専用スコープとはいえ、公開サイトに埋め込む場合は漏えい時にローテーションできる運用にしておいてください。

## 認証の仕組み

ブラウザの `WebSocket` API は任意の HTTP ヘッダを付与できないため、認証は **サブプロトコル**(コンストラクタの第 2 引数)で行います。`thq` と `thq-auth-<トークン>` の 2 つを渡してください。

```ts
const ws = new WebSocket("wss://thq.example.com/ws", [
  "thq",
  `thq-auth-${observerToken}`,
]);
```

認証に成功するとサーバーは `Sec-WebSocket-Protocol: thq` を返して接続が確立します。トークンが欠けている・間違っている場合はハンドシェイクが 401 で失敗し、ブラウザからは即座の `close` イベントとして観測されます(ステータスコードは JavaScript からは読めません)。

## プロトコル

### 1. 購読開始

接続後、`subscribe` メッセージを 1 回送ります。`device` は観測側の識別名で、サーバーのログに記録されます。

```json
{ "type": "subscribe", "device": "web-dashboard" }
```

### 2. スナップショット → リアルタイム配信

購読直後に、サーバーがメモリ上に保持している直近のイベント(リングバッファ、デフォルト 1000 件)がまとめて送られてきます。その後は新しいイベントが発生するたびにリアルタイムで配信されます。

再接続するとスナップショットが再送されるため、クライアント側では **`id` による重複排除**を入れておくと堅牢です。

### 3. 受信メッセージの形式

すべてテキストフレームの JSON で、`type` フィールドで種別を判定します(フィールド名は snake_case です)。

**location_update** — `sendLocation` Mutation で登録された位置情報

```json
{
  "type": "location_update",
  "id": "uuid",
  "session_id": "client-generated-session-id",
  "device": "device-001",
  "state": "arrived | approaching | passing | moving",
  "station_id": 1130201,
  "line_id": 11302,
  "coords": { "latitude": 35.6812, "longitude": 139.7671, "accuracy": 5.0, "speed": 45.0 },
  "timestamp": 1706000000000,
  "segment_id": "11302:1130201:1130202",
  "from_station_id": 1130201,
  "to_station_id": 1130202,
  "battery_level": 0.85,
  "battery_state": 2
}
```

`segment_id` / `from_station_id` / `to_station_id` はサーバー側の区間推定によって付与されます(トポロジ未設定時や推定不能時は `null`)。

**log** — `sendLogEvent` Mutation で登録されたログ

```json
{
  "type": "log",
  "id": "uuid",
  "session_id": "client-generated-session-id",
  "device": "device-001",
  "app_version": "1.2.3",
  "platform": "ios | android | macos | unknown",
  "channel": "production | canary",
  "timestamp": 1706000000000,
  "log": { "type": "system | app | client", "level": "debug | info | warn | error", "message": "..." }
}
```

**interaction** — `sendInteractionEvent` Mutation で登録されたユーザーインタラクション

```json
{
  "type": "interaction",
  "id": "uuid",
  "session_id": "client-generated-session-id",
  "device": "device-001",
  "app_version": "1.2.3",
  "platform": "ios | android | macos | unknown",
  "channel": "production | canary",
  "timestamp": 1706000000000,
  "event_name": "tab_change",
  "properties": { "tab": "map", "index": 2, "pinned": true }
}
```

ログイベントとインタラクションイベントは匿名で送信できるため、`device` が `null` の場合があります(`location_update` の `device` は常に非 null です)。

**error** — プロトコルエラーの通知(不正な JSON を送った場合など)

```json
{
  "type": "error",
  "error": { "type": "websocket_message_error | json_parse_error", "reason": "..." }
}
```

なお、バイナリフレームはサーバーに拒否されます。Ping/Pong はブラウザが自動で処理するため意識する必要はありません。

## TanStack Query との組み合わせ

WebSocket の受信データを `queryClient.setQueryData` でキャッシュに書き込み、表示側は通常の `useQuery` で読む構成が定石です。フェッチは発生しないので `staleTime: Infinity` にします。

```ts
// hooks/useTelemetryFeed.ts
import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

export interface LocationUpdateEvent {
  type: "location_update";
  id: string; // サーバー採番のイベント ID(重複排除に使える)
  session_id: string; // クライアント側で生成されたセッション ID
  device: string;
  state: "arrived" | "approaching" | "passing" | "moving";
  station_id: number | null;
  line_id: number;
  coords: {
    latitude: number;
    longitude: number;
    accuracy: number | null;
    speed: number | null;
  };
  timestamp: number;
  segment_id: string | null;
  from_station_id: number | null;
  to_station_id: number | null;
  battery_level: number | null;
  battery_state: 0 | 1 | 2 | 3 | null; // 0: UNKNOWN, 1: UNPLUGGED, 2: CHARGING, 3: FULL
}

export interface LogEvent {
  type: "log";
  id: string; // サーバー採番のイベント ID(重複排除に使える)
  session_id: string; // クライアント側で生成されたセッション ID
  device: string | null; // 匿名送信されたイベントは null
  app_version: string;
  platform: "ios" | "android" | "macos" | "unknown";
  channel: "production" | "canary";
  timestamp: number;
  log: {
    type: "system" | "app" | "client";
    level: "debug" | "info" | "warn" | "error";
    message: string;
  };
}

export interface InteractionEvent {
  type: "interaction";
  id: string; // サーバー採番のイベント ID(重複排除に使える)
  session_id: string; // クライアント側で生成されたセッション ID
  device: string | null; // 匿名送信されたイベントは null
  app_version: string;
  platform: "ios" | "android" | "macos" | "unknown";
  channel: "production" | "canary";
  timestamp: number;
  event_name: string; // 例: "app_launch", "tts_request"
  properties: Record<string, string | number | boolean | null> | null; // 付随情報(フラットなオブジェクトのみ)
}

export type TelemetryEvent = LocationUpdateEvent | LogEvent | InteractionEvent;

const FEED_KEY = ["telemetryFeed"] as const;
const MAX_EVENTS = 1000;
const MAX_RETRIES = 10;

export function useTelemetryFeed(
  wsUrl: string,
  observerToken: string,
  device = "web-dashboard",
) {
  const queryClient = useQueryClient();

  useEffect(() => {
    let ws: WebSocket | null = null;
    let retryTimer: ReturnType<typeof setTimeout>;
    let retries = 0;
    let disposed = false;

    const connect = () => {
      ws = new WebSocket(wsUrl, ["thq", `thq-auth-${observerToken}`]);

      ws.onopen = () => {
        retries = 0;
        ws?.send(JSON.stringify({ type: "subscribe", device }));
      };

      ws.onmessage = (event) => {
        const msg = JSON.parse(event.data);
        if (msg.type !== "location_update" && msg.type !== "log" && msg.type !== "interaction") {
          if (msg.type === "error") {
            console.warn("thq-server error:", msg.error);
          }
          return;
        }

        queryClient.setQueryData<TelemetryEvent[]>(FEED_KEY, (prev = []) => {
          // 再接続時のスナップショット再送に備えて id で重複排除する
          if (prev.some((e) => e.id === msg.id)) {
            return prev;
          }
          return [...prev, msg].slice(-MAX_EVENTS);
        });
      };

      ws.onclose = () => {
        // 認証失敗(401)もハンドシェイク失敗としてここに来るため、
        // 無限リトライにならないよう回数上限と指数バックオフを入れる
        if (disposed || retries >= MAX_RETRIES) return;
        const delay = Math.min(1000 * 2 ** retries, 30_000);
        retries += 1;
        retryTimer = setTimeout(connect, delay);
      };
    };

    connect();

    return () => {
      disposed = true;
      clearTimeout(retryTimer);
      ws?.close();
    };
  }, [wsUrl, observerToken, device, queryClient]);

  return useQuery<TelemetryEvent[]>({
    queryKey: FEED_KEY,
    queryFn: () => [],
    initialData: [],
    staleTime: Infinity, // データは WebSocket 側から書き込むため再フェッチ不要
    gcTime: Infinity,
  });
}
```

### 使用例

```tsx
function LiveMap() {
  const { data: events } = useTelemetryFeed(
    import.meta.env.VITE_THQ_WS_URL,
    import.meta.env.VITE_THQ_OBSERVER_TOKEN,
  );

  // 端末ごとの最新位置だけを表示する
  const latestByDevice = new Map<string, LocationUpdateEvent>();
  for (const e of events) {
    if (e.type === "location_update") {
      latestByDevice.set(e.device, e);
    }
  }

  return (
    <ul>
      {[...latestByDevice.values()].map((e) => (
        <li key={e.device}>
          {e.device}: ({e.coords.latitude.toFixed(4)}, {e.coords.longitude.toFixed(4)}) [{e.state}]
          {e.segment_id && ` 区間: ${e.segment_id}`}
        </li>
      ))}
    </ul>
  );
}
```

ログだけを流したい場合も同じフィードから絞り込めます。

```tsx
const logs = events.filter((e): e is LogEvent => e.type === "log");
```

## 環境変数(Vite の場合)

```bash
# .env.local (フロントエンド側リポジトリ)
VITE_THQ_WS_URL=wss://thq.example.com/ws
VITE_THQ_OBSERVER_TOKEN=<観測用トークン>
```

## 運用上の注意

- **切断は日常的に起きる**ものとして扱ってください。上記フックのようにバックオフ付きで再接続し、スナップショット再送は `id` の重複排除で吸収します。
- リングバッファの件数はサーバー側の `ring_size`(デフォルト 1000)で決まります。接続前のイベントをそれ以上さかのぼることはできないため、履歴が必要な場合は GraphQL の `accuracyByLine`(集計)や DB を参照してください。
- WebSocket は CORS の制約を受けないため、GraphQL と違いリバースプロキシなしでクロスオリジン接続できます。ただし `wss://`(TLS)を使わないと HTTPS ページからは接続できません(混在コンテンツとしてブロックされます)。
