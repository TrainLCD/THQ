import { listen } from "@tauri-apps/api/event";
import { z } from "zod";

export const MovingState = z.enum([
  "arrived",
  "approaching",
  "passing",
  "moving",
]);
export type MovingState = z.infer<typeof MovingState>;

export const LocationData = z.object({
  id: z.string(),
  lat: z.number().finite().min(-90).max(90).nullable(),
  lon: z.number().finite().min(-180).max(180).nullable(),
  accuracy: z.number().finite().nonnegative().nullable(),
  speed: z.number().finite().nonnegative().nullable(),
  timestamp: z.number().int().nonnegative(),
  state: MovingState,
  device: z.string(),
});
export type LocationData = z.infer<typeof LocationData>;

export const ErrorData = z.object({
  type: z.enum(["accuracy_low", "invalid_coords", "unknown"]),
  // TODO: 後で考える
  raw: z.unknown(),
});
export type ErrorData = z.infer<typeof ErrorData>;

export const LogData = z.object({
  id: z.string(),
  // system: THQサーバーが発行したログ
  // app: TrainLCDアプリが発行したログ
  // client: THQクライアントが発行したログ
  type: z.enum(["system", "app", "client"]).optional(),
  timestamp: z.number(),
  level: z.enum(["debug", "info", "warn", "error"]),
  message: z.string(),
  device: z.string(),
});
export type LogData = z.infer<typeof LogData>;

export const TelemetryEvent = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("location_update"),
    data: LocationData,
  }),
  z.object({
    type: z.literal("error"),
    data: ErrorData,
  }),
  z.object({
    type: z.literal("log"),
    data: LogData,
  }),
]);
export type TelemetryEvent = z.infer<typeof TelemetryEvent>;

export function registerTelemetryListener(handlers: {
  onLocationUpdate?: (data: LocationData) => void;
  onError?: (error: ErrorData) => void;
  onLog?: (log: LogData) => void;
}) {
  return listen<TelemetryEvent>("telemetry", (event) => {
    const parsedEvent = TelemetryEvent.safeParse(event.payload);
    if (!parsedEvent.success) {
      console.error("Invalid telemetry event", parsedEvent.error);
      handlers.onError?.({ type: "unknown", raw: parsedEvent.error });
      return;
    }
    const payload = parsedEvent.data;

    switch (payload.type) {
      case "location_update": {
        const parsed = LocationData.safeParse(payload.data);
        if (parsed.success) {
          handlers.onLocationUpdate?.(parsed.data);
          return;
        }
        console.error("Invalid location data", parsed.error);
        handlers.onError?.({
          type: "unknown",
          raw: parsed.error,
        });
        break;
      }
      case "error": {
        const parsed = ErrorData.safeParse(payload.data);
        if (parsed.success) {
          handlers.onError?.(parsed.data);
          return;
        }
        console.error("Invalid error data", parsed.error);
        handlers.onError?.({ type: "unknown", raw: parsed.error });
        break;
      }

      case "log": {
        const parsed = LogData.safeParse(payload.data);
        if (parsed.success) {
          handlers.onLog?.(parsed.data);
          return;
        }
        console.error("Invalid log data", parsed.error);
        handlers.onError?.({
          type: "unknown",
          raw: parsed.error,
        });
        break;
      }
    }
  });
}
