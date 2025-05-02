import { listen } from "@tauri-apps/api/event";
import { z } from "zod";

export type TelemetryEvent =
	| { type: "location_update"; data: LocationData }
	| { type: "error"; data: ErrorData };

export const MovingState = z.enum([
	"arrived",
	"approaching",
	"passing",
	"moving",
]);
export type MovingState = z.infer<typeof MovingState>;

export const LocationData = z.object({
	id: z.string(),
	lat: z.number(),
	lon: z.number(),
	accuracy: z.number().nullable(),
	speed: z.number(),
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

export function registerTelemetryListener(handlers: {
	onLocationUpdate?: (data: LocationData) => void;
	onError?: (error: ErrorData) => void;
}) {
	listen<TelemetryEvent>("telemetry", (event) => {
		const payload = event.payload;

		switch (payload.type) {
			case "location_update":
				handlers.onLocationUpdate?.(payload.data);
				break;
			case "error":
				handlers.onError?.(payload.data);
				break;
		}
	});
}
