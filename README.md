# thq-server

A telemetry server for [TrainLCD](https://github.com/TrainLCD). It provides real-time event streaming via WebSocket, a REST API for data ingestion, and a GraphQL API for aggregated reporting — all backed by optional PostgreSQL persistence.

## Features

- **WebSocket** — Real-time broadcast of location updates and log events
- **REST API** — Location ingestion (`POST /api/location`) and log submission (`POST /api/log`)
- **GraphQL** — Aggregated per-line accuracy reports (`POST /graphql`)
- **PostgreSQL persistence** — Optionally stores all events in the database
- **Ring buffer** — Keeps the latest N events in memory (default 1000)
- **Authentication** — WebSocket subprotocol-based auth; REST Bearer token auth
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

# With WebSocket auth
THQ_WS_AUTH_TOKEN=secret cargo run -- --host 0.0.0.0 --port 8080
```

### Docker Compose

```bash
# Set the auth token in .env
echo 'THQ_WS_AUTH_TOKEN=your-secret' > .env

# Build & start (includes PostgreSQL)
docker compose up --build
```

Endpoints after startup:

| Endpoint | URL |
|---|---|
| WebSocket | `ws://localhost:8080/ws` |
| REST API | `http://localhost:8080/api/location`, `/api/log` |
| GraphQL Playground | `http://localhost:8080/graphql` |
| Health check | `http://localhost:8080/healthz` |

## Configuration

Values can be set via CLI arguments, environment variables, or a config file (`config.toml`).

```toml
host = "0.0.0.0"
port = 8080
ring_size = 1000
database_url = "postgres://user:pass@localhost:5432/thq"
ws_auth_token = "change-me"
ws_auth_required = true
```

| Key | Environment variable | Default | Description |
|---|---|---|---|
| `host` | — | `127.0.0.1` | Bind address |
| `port` | — | `8080` | Listen port |
| `ring_size` | — | `1000` | Ring buffer capacity |
| `database_url` | `DATABASE_URL` | — | PostgreSQL connection URL |
| `ws_auth_token` | `THQ_WS_AUTH_TOKEN` | — | Auth token |
| `ws_auth_required` | `THQ_WS_AUTH_REQUIRED` | `true`* | Require authentication |

\* Defaults to `true` when a token is configured.

## API

### REST API

Authenticated endpoints require an `Authorization: Bearer <token>` header.

See [`openapi.yaml`](./openapi.yaml) for the full specification.

#### `POST /api/location` — Submit a location update

```json
{
  "device": "device-001",
  "state": "moving",
  "lineId": 11302,
  "coords": {
    "latitude": 35.6812,
    "longitude": 139.7671,
    "accuracy": 10.0,
    "speed": 45.0
  },
  "timestamp": 1706000000000
}
```

#### `POST /api/log` — Submit a log entry

```json
{
  "device": "device-001",
  "timestamp": 1706000000000,
  "log": {
    "type": "app",
    "level": "info",
    "message": "GPS signal acquired"
  }
}
```

#### `GET /healthz` — Health check

No authentication required. Returns `200 OK` if the server is running.

### WebSocket

Endpoint: `ws://<host>:<port>/ws`

Once connected, the server broadcasts `location_update` and `log` messages in real time.

#### Authentication

Send the token via WebSocket subprotocols:

```text
Sec-WebSocket-Protocol: thq, thq-auth-<token>
```

On success the server responds with `Sec-WebSocket-Protocol: thq`. When `ws_auth_required` is `true`, a missing or invalid token results in HTTP 401.

Set `ws_auth_required = false` to skip authentication during local development.

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
  "device": "device-id",
  "timestamp": 1234567890,
  "log": {
    "type": "system | app | client",
    "level": "debug | info | warn | error",
    "message": "System operational"
  }
}
```

**error**

```json
{
  "type": "error",
  "error": {
    "type": "websocket_message_error | json_parse_error | payload_parse_error | accuracy_low | invalid_coords | unknown",
    "reason": "..."
  }
}
```

### GraphQL

Endpoint: `POST /graphql` (Playground: `GET /graphql`)

Returns aggregated accuracy metrics per line. Raw location data is never exposed.

```graphql
query {
  accuracyByLine(
    lineId: "45"
    from: "2024-12-01T00:00:00Z"
    to: "2024-12-03T00:00:00Z"
    bucketSize: HOUR
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
| `bucketSize` | `TimeBucketSize!` | `MINUTE`, `HOUR`, or `DAY` |
| `limit` | `Int` | Max buckets returned (default 500, cap 2000) |

Maximum time span per bucket size: MINUTE ≤ 7 days, HOUR ≤ 90 days, DAY ≤ 365 days.

## Persistence

When `database_url` / `DATABASE_URL` is provided, the server connects to PostgreSQL, auto-creates tables, and stores every event.

| Table | Key columns |
|---|---|
| `location_logs` | `id`, `device`, `state`, `station_id`, `line_id`, `segment_id`, `from_station_id`, `to_station_id`, `latitude`, `longitude`, `accuracy`, `speed`, `battery_level`, `battery_state`, `timestamp`, `recorded_at` |
| `log_events` | `id`, `device`, `log_type`, `log_level`, `message`, `timestamp`, `recorded_at` |

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
