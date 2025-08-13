import { useAtom } from "jotai";
import uniqBy from "lodash/uniqBy";
import { useCallback, useEffect, useState } from "react";
import { telemetryListAtom } from "~/atoms/telemetryItem";
import {
	type ErrorData,
	type LocationData,
	type LogData,
	registerTelemetryListener,
} from "~/domain/commands";
import { isLocalServerEnabledAsync } from "~/utils/server";

export const useTelemetry = () => {
	const [isLocalServerAvailable, setIsLocalServerAvailable] = useState(false);
	const [error, setError] = useState<ErrorData | null>(null);
	const [consoleLogs, setConsoleLogs] = useState<LogData[]>([]);
	const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

	const handleLocationUpdate = useCallback(
		(data: LocationData) => {
			setTelemetryList((prev) => uniqBy([...prev, { ...data }], "id"));
		},
		[setTelemetryList],
	);

	useEffect(() => {
		const updateServerAvailabilityAsync = async () => {
			setIsLocalServerAvailable(await isLocalServerEnabledAsync());
		};
		updateServerAvailabilityAsync();
	}, []);

	useEffect(() => {
		registerTelemetryListener({
			onLocationUpdate: handleLocationUpdate,
			onError: (err) => setError(err),
			onLog: (log) => {
				setConsoleLogs((prev) => uniqBy([...prev, log], "id"));
			},
		});
	}, [handleLocationUpdate]);

	return {
		telemetryList,
		error,
		consoleLogs,
		isLocalServerAvailable,
	};
};
