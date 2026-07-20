# thq-server

A telemetry server for [TrainLCD](https://github.com/TrainLCD). It provides real-time event streaming via WebSocket and a GraphQL API for data ingestion and aggregated reporting — all backed by optional PostgreSQL persistence.

## Features

- **WebSocket** — Real-time broadcast of location updates, log events, and interaction events
- **GraphQL** — Event ingestion (`sendLogEvent`, `sendInteractionEvent`, `sendLocation` mutations), history queries (`logEvents`, `interactionEvents`, `locations`) and aggregated per-line accuracy reports (`POST /graphql`)
- **PostgreSQL persistence** — Optionally stores all events in the database
- **Ring buffer** — Keeps the latest N events in memory (default 1000)
- **Scoped authentication** — Three shared secrets: observer (WebSocket + history queries), events (log + interaction submission), telemetry (log + interaction + location submission)
- **Line topology** — Automatic segment annotation from a CSV topology file

## Requirements

- **Rust 1.91+** (pinned via `rust-toolchain.toml`)
- **PostgreSQL 18** (if persistence is enabled)
- **Docker / Docker Compose** (for containerized deployment)

## Quick start

### Local

```bash
# Basic startup
cargo run -- --host 0.0.0.0 --port 8080

# With a config file
cargo run -- --config config.toml

# With PostgreSQL persistence
cargo run -- --database-url postgres://user:pass@localhost:5432/thq

# With authentication
THQ_OBSERVER_AUTH_TOKEN=obs-secret \
THQ_EVENTS_AUTH_TOKEN=events-secret \
THQ_TELEMETRY_AUTH_TOKEN=telemetry-secret \
cargo run -- --host 0.0.0.0 --port 8080
```

### Docker Compose

```bash
# Set the auth tokens in .env (see .env.example)
cp .env.example .env

# Build & start (includes PostgreSQL)
docker compose up --build
```

Endpoints after startup:

| Endpoint | URL |
|---|---|
| WebSocket | `ws://localhost:8080/ws` |
| GraphQL | `http://localhost:8080/graphql` (POST) |
| Health check | `http://localhost:8080/healthz` |

## Configuration

Values can be set via CLI arguments, environment variables, or a config file (`config.toml`).

```toml
host = "0.0.0.0"
port = 8080
ring_size = 1000
database_url = "postgres://user:pass@localhost:5432/thq"
observer_auth_token = "change-me-observer"
events_auth_token = "change-me-events"
telemetry_auth_token = "change-me-telemetry"
```

| Key | Environment variable | Default | Description |
|---|---|---|---|
| `host` | — | `127.0.0.1` | Bind address |
| `port` | — | `8080` | Listen port |
| `ring_size` | — | `1000` | Ring buffer capacity |
| `database_url` | `DATABASE_URL` | — | PostgreSQL connection URL |
| `observer_auth_token` | `THQ_OBSERVER_AUTH_TOKEN` | — | Token for WebSocket observers |
| `events_auth_token` | `THQ_EVENTS_AUTH_TOKEN` | — | Token allowed to send log and interaction events |
| `telemetry_auth_token` | `THQ_TELEMETRY_AUTH_TOKEN` | — | Token allowed to send log/interaction events **and** location updates |

## Authentication

Three shared secrets grant exactly one role each:

| Token | WebSocket subscribe | History queries (`logEvents` / `interactionEvents` / `locations`) | `sendLogEvent` / `sendInteractionEvent` | `sendLocation` |
|---|---|---|---|---|
| Observer | ✅ | ✅ | ❌ | ❌ |
| Events | ❌ | ❌ | ✅ | ❌ |
| Telemetry | ❌ | ❌ | ✅ | ✅ |

- **WebSocket** — send the observer token via subprotocols: `Sec-WebSocket-Protocol: thq, thq-auth-<token>`
- **GraphQL mutations** — send the events or telemetry token via `Authorization: Bearer <token>`
- **GraphQL history queries** — send the observer token via `Authorization: Bearer <token>`; raw event data is exposed only to the observation role that already sees it in real time over WebSocket
- **GraphQL aggregated queries** (`accuracyByLine`) — no authentication (aggregated data only)

Authentication is always enforced. At least one token must be configured, or the server refuses to start.

## API

### GraphQL

Endpoint: `POST /graphql`

#### `sendLogEvent` — Submit a log event

Requires the events token or the telemetry token.

```graphql
mutation {
  sendLogEvent(input: {
    sessionId: "d0f7..."   # client-generated unique session identifier
    device: "device-001"   # optional — omit to submit anonymously
    appVersion: "1.2.3"
    platform: ios      # ios | android | macos | unknown
    channel: production    # production | canary
    timestamp: 1706000000000
    type: app          # system | app | client
    level: info        # debug | info | warn | error
    message: "GPS signal acquired"
  }) {
    sessionId
  }
}
```

`sessionId` is a mandatory, client-generated unique identifier (any string). Event IDs are always generated server-side. `device` is optional so that log events can be submitted anonymously; omitted values are broadcast and stored as `null`.

#### `sendInteractionEvent` — Record a user-driven interaction

Requires the events token or the telemetry token (any token except the observer one). Unlike `sendLogEvent`, which carries console.* output, this records a named user action such as an app launch, tab change, TTS request result, or feedback submission result.

```graphql
mutation {
  sendInteractionEvent(input: {
    sessionId: "d0f7..."   # client-generated unique session identifier
    device: "device-001"   # optional — omit to submit anonymously
    appVersion: "1.2.3"
    platform: ios      # ios | android | macos | unknown
    channel: production    # production | canary
    timestamp: 1706000000000
    eventName: "tab_change"    # arbitrary event name
    properties: { tab: "map", index: 2, pinned: true }   # optional flat map
  }) {
    sessionId
  }
}
```

`properties` is an optional flat object — the TS equivalent is `Record<string, string | number | boolean | null>`. Nested objects and arrays are rejected.

#### `sendLocation` — Submit a location update

Requires the telemetry token; the events token is deliberately not enough to publish positional data. Unlike `sendLogEvent`, `device` is mandatory here. The update is validated, annotated with segment information, broadcast to WebSocket subscribers, and persisted.

```graphql
mutation {
  sendLocation(input: {
    sessionId: "d0f7..."   # client-generated unique session identifier
    device: "device-001"
    state: moving      # arrived | approaching | passing | moving
    lineId: 11302
    coords: {
      latitude: 35.6812
      longitude: 139.7671
      accuracy: 10.0
      speed: 45.0
    }
    timestamp: 1706000000000
  }) {
    sessionId
    warning   # set when e.g. the reported accuracy exceeds 100 m
  }
}
```

`stationId` is only meaningful when `state` is `arrived` or `passing` and is ignored otherwise. `batteryLevel` (0.0–1.0) and `batteryState` (`unknown | unplugged | charging | full`) are optional.

#### `logEvents` / `interactionEvents` / `locations` — History queries

Each mutation has a matching query returning the persisted events, newest first. All three require the **observer token** (`Authorization: Bearer <token>`) — the same read-only role that observes events in real time over WebSocket — and a configured database.

```graphql
query {
  logEvents(
    sessionId: "d0f7..."       # all filters are optional
    device: "device-001"
    from: "2026-07-01T00:00:00Z"   # client-reported timestamp range
    to: "2026-07-02T00:00:00Z"
    type: app                  # system | app | client
    level: error               # debug | info | warn | error
    limit: 100                 # default 100, cap 2000
  ) {
    id sessionId device appVersion platform channel
    timestamp type level message recordedAt
  }
}
```

```graphql
query {
  interactionEvents(eventName: "tab_change", limit: 50) {
    id sessionId device appVersion platform channel
    timestamp eventName properties recordedAt
  }
}
```

```graphql
query {
  locations(lineId: 11302, state: moving, from: "2026-07-01T00:00:00Z", to: "2026-07-02T00:00:00Z") {
    id sessionId device state stationId lineId
    coords { latitude longitude accuracy speed }
    timestamp segmentId fromStationId toStationId
    batteryLevel batteryState recordedAt
  }
}
```

Shared parameters (all optional):

| Parameter | Type | Description |
|---|---|---|
| `sessionId` | `String` | Exact session ID match |
| `device` | `String` | Exact device ID match |
| `from` | `DateTime` | Inclusive lower bound on the client-reported timestamp |
| `to` | `DateTime` | Exclusive upper bound on the client-reported timestamp |
| `limit` | `Int` | Max events returned, newest first (default 100, cap 2000) |

Per-query filters: `logEvents` also accepts `type` and `level`; `interactionEvents` accepts `eventName`; `locations` accepts `lineId` and `state`.

Columns added to the storage schema over time are nullable in the results: legacy rows recorded before a column existed return `null` for it (e.g. `sessionId`, `appVersion`, or `lineId` on old rows). `recordedAt` is the server-side persistence time, while `timestamp` is the client-reported unix-millisecond value.

#### `accuracyByLine` — Aggregated accuracy report

Returns aggregated accuracy metrics per line. Raw event data is exposed only through the observer-token history queries above; this aggregated report requires no authentication.

```graphql
query {
  accuracyByLine(
    lineId: "45"
    from: "2024-12-01T00:00:00Z"
    to: "2024-12-03T00:00:00Z"
    bucketSize: hour
    limit: 100
  ) {
    lineId
    buckets {
      bucketStart
      bucketEnd
      avgAccuracy
      p90Accuracy
      sampleCount
    }
  }
}
```

| Parameter | Type | Description |
|---|---|---|
| `lineId` | `ID!` | Line ID |
| `from` | `DateTime!` | Start of the time range |
| `to` | `DateTime!` | End of the time range |
| `bucketSize` | `TimeBucketSize!` | `minute`, `hour`, or `day` |
| `limit` | `Int` | Max buckets returned (default 500, cap 2000) |

Maximum time span per bucket size: minute ≤ 7 days, hour ≤ 90 days, day ≤ 365 days.

#### `GET /healthz` — Health check

No authentication required. Returns `200 OK` if the server is running.

### WebSocket

Endpoint: `ws://<host>:<port>/ws`

Once connected, the server broadcasts `location_update`, `log` and `interaction` messages in real time. Authentication uses the observer token (see [Authentication](#authentication)); on success the server responds with `Sec-WebSocket-Protocol: thq`, while a missing or invalid token results in HTTP 401.

#### Message formats

**subscribe**

```json
{ "type": "subscribe", "device": "device-id" }
```

**location_update**

```json
{
  "id": "uuid",
  "type": "location_update",
  "session_id": "client-generated-session-id",
  "device": "device-id",
  "state": "arrived | approaching | passing | moving",
  "station_id": 123,
  "line_id": 45,
  "coords": {
    "latitude": 35.0,
    "longitude": 139.0,
    "accuracy": 5.0,
    "speed": 10.0
  },
  "timestamp": 1234567890
}
```

**log**

```json
{
  "id": "uuid",
  "type": "log",
  "session_id": "client-generated-session-id",
  "device": "device-id",
  "app_version": "1.2.3",
  "platform": "ios | android | macos | unknown",
  "channel": "production | canary",
  "timestamp": 1234567890,
  "log": {
    "type": "system | app | client",
    "level": "debug | info | warn | error",
    "message": "System operational"
  }
}
```

`device` is `null` when the event was submitted anonymously.

**interaction**

```json
{
  "id": "uuid",
  "type": "interaction",
  "session_id": "client-generated-session-id",
  "device": "device-id",
  "app_version": "1.2.3",
  "platform": "ios | android | macos | unknown",
  "channel": "production | canary",
  "timestamp": 1234567890,
  "event_name": "tab_change",
  "properties": { "tab": "map", "index": 2, "pinned": true }
}
```

As with `log`, `device` is `null` when the event was submitted anonymously.

**error**

```json
{
  "type": "error",
  "error": {
    "type": "websocket_message_error | json_parse_error",
    "reason": "..."
  }
}
```

## Persistence

When `database_url` / `DATABASE_URL` is provided, the server connects to PostgreSQL, auto-creates tables, and stores every event.

| Table | Key columns |
|---|---|
| `location_logs` | `id`, `session_id`, `device`, `state`, `station_id`, `line_id`, `segment_id`, `from_station_id`, `to_station_id`, `latitude`, `longitude`, `accuracy`, `speed`, `battery_level`, `battery_state`, `timestamp`, `recorded_at` |
| `log_events` | `id`, `session_id`, `device`, `app_version`, `platform`, `channel`, `log_type`, `log_level`, `message`, `timestamp`, `recorded_at` |
| `interaction_events` | `id`, `session_id`, `device`, `app_version`, `platform`, `channel`, `properties` (JSONB), `event_name`, `timestamp`, `recorded_at` |

Without a `database_url` the server still accepts WebSocket traffic but does not persist messages.

## Project structure

```text
src/
├── main.rs       # Entrypoint
├── config.rs     # CLI arguments & config file parsing
├── server.rs     # Axum HTTP / WebSocket server
├── state.rs      # Shared application state
├── domain.rs     # Domain model definitions
├── storage.rs    # PostgreSQL persistence layer
├── graphql.rs    # GraphQL schema & resolvers
├── segment.rs    # Line topology & segment inference
└── static/
    └── join.csv  # Line topology data
```

## License

[MIT License](./LICENSE)
