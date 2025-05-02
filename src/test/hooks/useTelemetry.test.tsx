/**
 * @vitest-environment jsdom
 */
import { act, renderHook } from "@testing-library/react";
import { Provider, createStore } from "jotai";
import { vi } from "vitest";
import { useTelemetry } from "../../hooks/useTelemetry";

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
		const store = createStore();
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
				id: "test-id",
				lat: 35,
				lon: 139,
				accuracy: 5,
				speed: 1.2,
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
		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		let errorHandler: ((err: any) => void) | undefined;

		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		(registerTelemetryListener as any).mockImplementation(
			// biome-ignore lint/suspicious/noExplicitAny: <explanation>
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

	it("truncates telemetry list to last 1,000 items", () => {
		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		let locationHandler: ((data: any) => void) | undefined;

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
			for (let i = 0; i < 1050; i++) {
				locationHandler?.({
					id: "test-id",
					lat: 35,
					lon: 139,
					accuracy: 5,
					speed: 1,
					timestamp: i, // ユニークな値で順序確認用
				});
			}
		});

		const list = result.current.telemetryList;

		expect(list).toHaveLength(1000);
		expect(list[0].timestamp).toBe(50); // 最古が先頭
		expect(list[999].timestamp).toBe(1049); // 最新が最後尾
	});
});
