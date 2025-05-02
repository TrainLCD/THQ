/**
 * @vitest-environment jsdom
 */
import { renderHook, act } from "@testing-library/react";
import { useTelemetry } from "../../hooks/useTelemetry";
import { getDefaultStore, Provider } from "jotai";
import { vi } from "vitest";

vi.mock("../../domain/commands", async () => {
	return {
		registerTelemetryListener: vi.fn(),
	};
});

import {
	type LocationData,
	registerTelemetryListener,
} from "../../domain/commands";

describe("useTelemetry", () => {
	const createWrapper = () => {
		const store = getDefaultStore();
		return ({ children }: { children: React.ReactNode }) => (
			<Provider store={store}>{children}</Provider>
		);
	};

	beforeEach(() => {
		vi.resetAllMocks();
	});

	it("adds location data to telemetryList", () => {
		let locationHandler: ((data: LocationData) => void) | undefined;

		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		(registerTelemetryListener as any).mockImplementation(
			// biome-ignore lint/suspicious/noExplicitAny: <explanation>
			({ onLocationUpdate }: any) => {
				locationHandler = onLocationUpdate;
			},
		);

		const { result } = renderHook(() => useTelemetry(), {
			wrapper: createWrapper(),
		});

		act(() => {
			locationHandler?.({
				lat: 35,
				lon: 139,
				accuracy: 5,
				speed: 1.2,
				gForce: 1.01,
				timestamp: 1234567890,
			});
		});

		expect(result.current.telemetryList).toHaveLength(1);
		expect(result.current.telemetryList[0]).toMatchObject({
			lat: 35,
			lon: 139,
		});
	});

	it("sets error when onError is called", () => {
		let errorHandler: ((err: any) => void) | undefined;

		(registerTelemetryListener as any).mockImplementation(
			({ onError }: any) => {
				errorHandler = onError;
			},
		);

		const { result } = renderHook(() => useTelemetry(), {
			wrapper: createWrapper(),
		});

		act(() => {
			errorHandler?.({
				type: "accuracy_low",
				raw: { detail: "accuracy too low" },
			});
		});

		expect(result.current.error).toMatchObject({
			type: "accuracy_low",
		});
	});

	it("truncates telemetry list to last 100 items", () => {
		let locationHandler: ((data: LocationData) => void) | undefined;

		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		(registerTelemetryListener as any).mockImplementation(
			// biome-ignore lint/suspicious/noExplicitAny: <explanation>
			({ onLocationUpdate }: any) => {
				locationHandler = onLocationUpdate;
			},
		);

		const { result } = renderHook(() => useTelemetry(), {
			wrapper: createWrapper(),
		});

		act(() => {
			for (let i = 0; i < 150; i++) {
				locationHandler?.({
					lat: 35 + i,
					lon: 139,
					accuracy: 5,
					speed: 1,
					gForce: 1,
					timestamp: i,
				});
			}
		});

		expect(result.current.telemetryList).toHaveLength(100);
		expect(result.current.telemetryList[0].lat).toBe(85); // 最初の50件は落ちているはず
	});
});
