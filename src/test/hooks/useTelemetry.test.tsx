/**
 * @vitest-environment jsdom
 */
import { act, renderHook } from "@testing-library/react";
import { Provider, createStore } from "jotai";
import { vi } from "vitest";
import { LocationData, registerTelemetryListener } from "~/domain/commands";
import { useTelemetry } from "~/hooks/useTelemetry";

vi.mock(import("~/domain/commands"), async (importOriginal) => {
	const mod = await importOriginal();
	return {
		LocationData: mod.LocationData,
		registerTelemetryListener: vi.fn(),
	};
});

const createWrapper = () => {
	const store = createStore();
	return ({ children }: { children: React.ReactNode }) => (
		<Provider store={store}>{children}</Provider>
	);
};

describe("useTelemetry (uniq + max 1000件)", () => {
	beforeEach(() => {
		vi.resetAllMocks();
	});

	it("adds unique location data and removes duplicates", () => {
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

		const sample = LocationData.safeParse({
			id: "loc-1",
			lat: 35,
			lon: 139,
			accuracy: 5,
			speed: 1,
			timestamp: 1000,
			state: "arrived",
			device: "test-device",
		});
		1;
		expect(sample.success).toBe(true);
		expect(sample.data).toBeDefined();

		act(() => {
			if (sample.data) {
				locationHandler?.(sample.data);
				locationHandler?.({ ...sample.data }); // 重複データ
			}
		});

		expect(result.current.telemetryList).toHaveLength(1); // 重複除外
		expect(result.current.telemetryList[0].id).toBe("loc-1");
	});

	it("maintains at most 1000 unique telemetry items", () => {
		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		let locationHandler: ((data: any) => void) | undefined;

		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		(registerTelemetryListener as any).mockImplementation(
			// biome-ignore lint/suspicious/noExplicitAny: <explanation>
			({ onLocationUpdate }: { onLocationUpdate: any }) => {
				locationHandler = onLocationUpdate;
			},
		);

		const { result } = renderHook(() => useTelemetry(), {
			wrapper: createWrapper(),
		});

		act(() => {
			for (let i = 0; i < 10050; i++) {
				const sample = LocationData.safeParse({
					id: `loc-${i}`,
					lat: 35,
					lon: 139,
					accuracy: 5,
					speed: 1,
					timestamp: i,
					state: "arrived",
					device: "test-device",
				});
				expect(sample.success).toBe(true);
				expect(sample.data).toBeDefined();

				if (sample.data) {
					locationHandler?.(sample.data);
				}
			}
		});

		const list = result.current.telemetryList;

		expect(list).toHaveLength(10000);
		expect(list[0].id).toBe("loc-50"); // 最初の50件は落ちる
		expect(list[9999].id).toBe("loc-10049"); // 最新が末尾に
	});

	it("sets error when onError is called", () => {
		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		let errorHandler: ((err: any) => void) | undefined;

		// biome-ignore lint/suspicious/noExplicitAny: <explanation>
		(registerTelemetryListener as any).mockImplementation(
			// biome-ignore lint/suspicious/noExplicitAny: <explanation>
			({ onError }: { onError: any }) => {
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

		expect(result.current.error?.type).toBe("accuracy_low");
	});
});
