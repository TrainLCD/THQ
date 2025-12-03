# thq-server

Rust-only telemetry WebSocket server for THQ/TrainLCD.

- WebSocket endpoint: `ws://<host>:<port>/ws`
- Health check: `GET /healthz`
- Broadcasts `location_update` and `log` messages to all subscribers
- Maintains a ring buffer of the latest N events (default 1000)

## Usage

```bash
cargo run -- --host 0.0.0.0 --port 8080
# or with config file
cargo run -- --config config.toml
# DATABASE_URL can also be supplied via env or config file
cargo run -- --database-url postgres://user:pass@localhost:5432/thq
```

Example `config.toml`:

```toml
host = "0.0.0.0"
port = 8080
ring_size = 1000
database_url = "postgres://user:pass@localhost:5432/thq"
```

## Persistence

When `database_url`/`DATABASE_URL` is provided, the server connects to PostgreSQL,
creates the tables if missing, and stores each incoming message:

- `location_events`: `id`, `device`, `state`, `latitude`, `longitude`, `accuracy`, `speed`, `timestamp`, `recorded_at`
- `log_events`: `id`, `device`, `log_type`, `log_level`, `message`, `timestamp`, `recorded_at`

Without a `database_url` the server still accepts WebSocket traffic but does not
persist messages.

## Message formats

### subscribe
```json
{"type": "subscribe", "device": "device-id"}
```

### location_update
```json
{
  "id": "uuid",
  "type": "location_update",
  "device": "device-id",
  "state": "arrived | approaching | passing | moving",
  "coords": {
    "latitude": 35.0,
    "longitude": 139.0,
    "accuracy": 5.0,
    "speed": 10.0
  },
  "timestamp": 1234567890
}
```

### log
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

### error
```json
{
  "type": "error",
  "error": {
    "type": "websocket_message_error | json_parse_error | payload_parse_error | accuracy_low | invalid_coords | unknown",
    "reason": "..."
  }
}
```
