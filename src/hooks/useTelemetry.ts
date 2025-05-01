import { useEffect, useState } from "react";
import {
	type ErrorData,
	type LocationData,
	registerTelemetryListener,
} from "../domain/commands";

export const useTelemetry = () => {
	const [location, setLocation] = useState<LocationData | null>(null);
	const [error, setError] = useState<ErrorData | null>(null);

	useEffect(() => {
		registerTelemetryListener({
			onLocationUpdate: setLocation,
			onError: setError,
		});
	}, []);

	return { location, error };
};
