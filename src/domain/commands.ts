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
  lat: z.number().nullable(),
  lon: z.number().nullable(),
  accuracy: z.number().nullable(),
  speed: z.number().nullable(),
  timestamp: z.number(),
  state: MovingState,
  device: z.string(),
});
export type LocationData = z.infer<typeof LocationData>;

export const ErrorData = z.object({
  type: z.enum(["accuracy_low", "invalid_coords", "unknown"]),
  // TODO: 後で考える
  raw: z.any(),
});
export type ErrorData = z.infer<typeof ErrorData>;

export const LogData = z.object({
  id: z.string(),
  type: z.enum(["log"]),
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
    const payload = event.payload;

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
      case "error":
        handlers.onError?.(payload.data);
        break;
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
