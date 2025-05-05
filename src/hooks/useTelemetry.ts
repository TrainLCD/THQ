import { useAtom } from "jotai";
import uniqBy from "lodash/uniqBy";
import { useCallback, useEffect, useState } from "react";
import { telemetryListAtom } from "~/atoms/telemetryItem";
import {
	type ErrorData,
	type LocationData,
	registerTelemetryListener,
} from "~/domain/commands";

export const useTelemetry = () => {
	const [error, setError] = useState<ErrorData | null>(null);

	const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

	const handleLocationUpdate = useCallback(
		(data: LocationData) => {
			setTelemetryList((prev) =>
				uniqBy(
					[
						...prev.slice(-999), // 最大1,000件まで保持
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
		});
	}, [handleLocationUpdate]);

	return { telemetryList, error };
};
