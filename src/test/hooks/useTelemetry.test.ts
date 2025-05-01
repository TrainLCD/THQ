import { act, renderHook } from "@testing-library/react";
/**
 * @vitest-environment jsdom
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ErrorData, LocationData } from "../../domain/commands";
import { useTelemetry } from "../../hooks/useTelemetry";

const mockRegisterTelemetryListener = vi.fn();

let fireLocationUpdate: ((data: LocationData) => void) | undefined;
let fireError: ((data: ErrorData) => void) | undefined;

vi.mock("../../domain/commands", () => ({
	registerTelemetryListener: vi.fn((handlers) => {
		fireLocationUpdate = handlers.onLocationUpdate;
		fireError = handlers.onError;
	}),
}));

describe("useTelemetry", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("updates location state on location_update", () => {
		const fakeLocation: LocationData = {
			lat: 35,
			lon: 139,
			accuracy: 5,
			speed: 10,
			gForce: 9.8,
			timestamp: 1234567890,
		};

		let onLocationUpdate: ((data: LocationData) => void) | undefined;

		mockRegisterTelemetryListener.mockImplementation(
			({ onLocationUpdate: cb }) => {
				onLocationUpdate = cb;
			},
		);

		const { result } = renderHook(() => useTelemetry());

		act(() => {
			fireLocationUpdate?.(fakeLocation);
		});

		expect(result.current.location).toEqual(fakeLocation);
		expect(result.current.error).toBe(null);
	});

	it("updates error state on error event", () => {
		const fakeError: ErrorData = {
			type: "accuracy_low",
			raw: { reason: "accuracy > 100" },
		};

		let onError: ((data: ErrorData) => void) | undefined;

		mockRegisterTelemetryListener.mockImplementation(({ onError: cb }) => {
			onError = cb;
		});

		const { result } = renderHook(() => useTelemetry());

		act(() => {
			fireError?.(fakeError);
		});

		expect(result.current.error).toEqual(fakeError);
		expect(result.current.location).toBe(null);
	});
});
