import { useCallback, useEffect, useState } from "react";
import {
	type ErrorData,
	type LocationData,
	registerTelemetryListener,
} from "../domain/commands";
import { useAtom } from "jotai";
import { telemetryListAtom } from "../atoms/telemetryItem";

export const useTelemetry = () => {
	const [error, setError] = useState<ErrorData | null>(null);

	const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

	const handleLocationUpdate = useCallback(
		(data: LocationData) => {
			setTelemetryList((prev) => [
				...prev.slice(-99), // 最大100件まで保持
				{ ...data },
			]);
		},
		[setTelemetryList],
	);

	useEffect(() => {
		registerTelemetryListener({
			onLocationUpdate: handleLocationUpdate,
			onError: setError,
		});
	}, [handleLocationUpdate]);

	return { telemetryList, error };
};
