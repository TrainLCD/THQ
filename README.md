# THQ - TrainLCD Telemetry Headquarters

A real-time telemetry monitoring application built with Tauri, React, and TypeScript. THQ provides a comprehensive dashboard for tracking and visualizing location data, speed metrics, and system logs through WebSocket connections.

## ✨ Features

- **Real-time Location Tracking**: Monitor GPS coordinates, accuracy, and movement states
- **Interactive Mapping**: Live map visualization with OpenStreetMap integration using Leaflet
- **Speed Analytics**: Real-time speed charts and movement state tracking
- **WebSocket Architecture**: Supports both server and client modes for distributed monitoring
- **Telemetry Logging**: Comprehensive logging system with different severity levels
- **Data Visualization**: Interactive charts and tables for telemetry analysis
- **Responsive UI**: Modern, dark-mode compatible interface built with Tailwind CSS

## 🏗️ Architecture

THQ can operate in two modes:

### Server Mode

- Hosts a WebSocket server on port 8080
- Receives telemetry data from multiple clients
- Distributes data to connected subscribers
- Ideal for central monitoring stations

### Client Mode

- Connects to a remote WebSocket server
- Receives and displays telemetry data
- Perfect for remote monitoring dashboards

## 🚀 Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (v18 or later)
- [Rust](https://rustup.rs/) (latest stable)
- [Tauri Prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites)

### Installation

1. Clone the repository:

```bash
git clone https://github.com/TrainLCD/THQ.git
cd THQ
```

2. Install dependencies:

```bash
npm install
```

3. Build and run in development mode:

```bash
npm run tauri dev
```

### Running in Server Mode

To start THQ as a WebSocket server:

```bash
npm run tauri dev -- --enable-server
```

### Running in Client Mode

1. Create a `.env.client.local` file:

```env
WEBSOCKET_ENDPOINT=ws://your-server:8080
```

2. Run in client mode (default):

```bash
npm run tauri dev
```

## 🛠️ Development

### Available Scripts

- `npm run dev` - Start Vite development server
- `npm run build` - Build the application
- `npm run tauri dev` - Run Tauri in development mode
- `npm run tauri build` - Build Tauri application for production
- `npm run test` - Run tests
- `npm run test:watch` - Run tests in watch mode
- `npm run check` - Run Biome linter and formatter

### Project Structure

```
src/
├── components/          # React components
│   ├── ConsoleLogTable.tsx
│   ├── CurrentLocationMap.tsx
│   ├── MovingLogTable.tsx
│   └── SpeedChart.tsx
├── domain/             # Business logic and types
│   ├── commands.ts     # Tauri commands and event types
│   ├── emoji.ts        # State emoji mappings
│   └── threshold.ts    # Configuration constants
├── hooks/              # Custom React hooks
│   └── useTelemetry.ts # Main telemetry data hook
├── atoms/              # Jotai state management
└── utils/              # Utility functions

src-tauri/
├── src/
│   ├── ws_server.rs    # WebSocket server implementation
│   ├── ws_client.rs    # WebSocket client implementation
│   ├── domain.rs       # Rust data structures
│   └── tauri_bridge.rs # Tauri event bridge
└── Cargo.toml
```

### Data Types

THQ handles three main types of telemetry events:

#### LocationData

```typescript
{
  id: string;
  lat: number | null;
  lon: number | null;
  accuracy: number | null;
  speed: number | null;
  timestamp: number;
  state: "arrived" | "approaching" | "passing" | "moving";
  device: string;
}
```

#### LogData

```typescript
{
  id: string;
  timestamp: number;
  level: "debug" | "info" | "warn" | "error";
  message: string;
  device: string;
}
```

#### ErrorData

```typescript
{
  type: "accuracy_low" | "invalid_coords" | "unknown";
  raw: any;
}
```

## 🧪 Testing

The project includes comprehensive tests for hooks and domain logic:

```bash
# Run all tests
npm run test

# Run tests in watch mode
npm run test:watch
```

## 🔧 Configuration

### WebSocket Protocol

THQ uses a JSON-based WebSocket protocol for communication:

**Subscribe to events:**

```json
{
  "type": "subscribe"
}
```

**Location update:**

```json
{
  "type": "location_update",
  "device": "device-id",
  "state": "moving",
  "coords": {
    "latitude": 35.0,
    "longitude": 139.0,
    "accuracy": 5.0,
    "speed": 10.0
  },
  "timestamp": 1234567890
}
```

**Log message:**

```json
{
  "type": "log",
  "device": "device-id",
  "timestamp": 1234567890,
  "log": {
    "level": "info",
    "message": "System operational"
  }
}
```

## 🛡️ Error Handling

THQ implements comprehensive error handling for:

- Invalid GPS coordinates
- Low accuracy readings
- WebSocket connection failures
- Malformed telemetry data

## 📊 Performance

- Maintains up to 1,000 telemetry records in memory
- Automatic deduplication of telemetry data
- Efficient real-time updates using Jotai state management
- Optimized rendering with React.memo and useMemo

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests (`npm run test`)
5. Format code (`npm run check`)
6. Commit your changes (`git commit -m 'Add amazing feature'`)
7. Push to the branch (`git push origin feature/amazing-feature`)
8. Open a Pull Request

## 📝 License

This project is part of the TrainLCD ecosystem. Please refer to the license file for details.

## 🔗 Related Projects

- [TrainLCD](https://github.com/TrainLCD/TrainLCD) - Main TrainLCD application
- [MobileApp](https://github.com/TrainLCD/MobileApp) - TrainLCD mobile application

## 📞 Support

For questions and support, please open an issue on GitHub or contact the TrainLCD team.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
