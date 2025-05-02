import { type Mock, beforeEach, describe, expect, it, vi } from "vitest";
import {
	ErrorData,
	LocationData,
	registerTelemetryListener,
} from "../../domain/commands";

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn(),
}));

import { listen } from "@tauri-apps/api/event";

describe("registerTelemetryListener (zod)", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("calls onLocationUpdate with valid data", () => {
		const onLocationUpdate = vi.fn();
		const onError = vi.fn();

		registerTelemetryListener({ onLocationUpdate, onError });

		const handler = (listen as unknown as Mock).mock.calls[0][1];

		const payload = {
			type: "location_update",
			data: {
				id: "test-id",
				lat: 35,
				lon: 139,
				accuracy: 5,
				speed: 10,
				gForce: 9.8,
				timestamp: 1000,
			},
		};

		const result = LocationData.safeParse(payload.data);
		expect(result.success).toBe(true);

		handler({ payload });

		expect(onLocationUpdate).toHaveBeenCalledOnce();
		expect(onLocationUpdate).toHaveBeenCalledWith(payload.data);
		expect(onError).not.toHaveBeenCalled();
	});

	it("calls onError with valid data", () => {
		const onLocationUpdate = vi.fn();
		const onError = vi.fn();

		registerTelemetryListener({ onLocationUpdate, onError });

		const handler = (listen as unknown as Mock).mock.calls[0][1];

		const payload = {
			type: "error",
			data: {
				type: "accuracy_low",
				raw: { coords: { accuracy: 200 } },
			},
		};

		const result = ErrorData.safeParse(payload.data);
		expect(result.success).toBe(true);

		handler({ payload });

		expect(onError).toHaveBeenCalledOnce();
		expect(onError).toHaveBeenCalledWith(payload.data);
		expect(onLocationUpdate).not.toHaveBeenCalled();
	});

	it("fails validation with invalid location data", () => {
		const invalidData = {
			id: "test-id",
			lat: "not a number",
			lon: 139,
			accuracy: 5,
			speed: 10,
			gForce: 9.8,
			timestamp: 1000,
		};

		const result = LocationData.safeParse(invalidData);
		expect(result.success).toBe(false);
		expect(result.error?.issues[0].path).toContain("lat");
	});

	it("fails validation with invalid error data", () => {
		const invalidError = {
			type: "totally_wrong",
			raw: {},
		};

		const result = ErrorData.safeParse(invalidError);
		expect(result.success).toBe(false);
		expect(result.error?.issues[0].path).toContain("type");
	});
});
