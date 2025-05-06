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

export const useTelemetry = () => {
	const [error, setError] = useState<ErrorData | null>(null);
	const [consoleLogs, setConsoleLogs] = useState<LogData[]>([]);

	const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

	const handleLocationUpdate = useCallback(
		(data: LocationData) => {
			setTelemetryList((prev) =>
				uniqBy(
					[
						...prev.slice(-9999), // 最大10,000件まで保持
						{ ...data },
					],
					"id",
				),
			);
		},
		[setTelemetryList],
	);

	useEffect(() => {
		registerTelemetryListener({
			onLocationUpdate: handleLocationUpdate,
			onError: setError,
			onLog: (log) => {
				setConsoleLogs((prev) => uniqBy([...prev, log], "id"));
			},
		});
	}, [handleLocationUpdate]);

	return { telemetryList, error, consoleLogs };
};
