import type { LatLngTuple } from "leaflet";
import { memo, useMemo } from "react";
import { ConsoleLogTable } from "./components/ConsoleLogTable";
import { CurrentLocationMap } from "./components/CurrentLocationMap";
import { MovingLogTable } from "./components/MovingLogTable";
import { SpeedChart } from "./components/SpeedChart";
import { STATE_ICONS } from "./domain/emoji";
import { useTelemetry } from "./hooks/useTelemetry";
import { toKMH } from "./utils/unit";
import getRhumbLineBearing from "geolib/es/getRhumbLineBearing";
import type { LocationData } from "./domain/commands";
import { BAD_ACCURACY_THRESHOLD } from "./domain/threshold";

function App() {
	const { telemetryList, error, consoleLogs } = useTelemetry();

	const latestTelemetry = useMemo(
		() => telemetryList[telemetryList.length - 1],
		[telemetryList],
	);
	const latestTelemetryBearing = useMemo(() => {
		const latest: LocationData | undefined =
			telemetryList[telemetryList.length - 1];
		const prev: LocationData | undefined =
			telemetryList[telemetryList.length - 2];
		if (!latest?.lat || !latest?.lon || !prev?.lat || !prev?.lon) return 0;
		return getRhumbLineBearing(
			{
				latitude: latest.lat,
				longitude: latest.lon,
			},
			{
				latitude: prev.lat,
				longitude: prev.lon,
			},
		);
	}, [telemetryList]);

	const badAccuracy = useMemo(() => {
		if (latestTelemetry?.accuracy === null) return false;
		if (latestTelemetry?.accuracy > BAD_ACCURACY_THRESHOLD) return true;
		return false;
	}, [latestTelemetry?.accuracy]);

	const movingLogs = useMemo(
		() => telemetryList.slice().sort((a, b) => b.timestamp - a.timestamp),
		[telemetryList],
	);

	const sortedConsoleLogs = useMemo(
		() => consoleLogs.slice().sort((a, b) => b.timestamp - a.timestamp),
		[consoleLogs],
	);

	const locations = useMemo<LatLngTuple[]>(
		() =>
			telemetryList
				.map((t) =>
					t.lat !== null && t.lon !== null
						? ([t.lat, t.lon] as LatLngTuple)
						: undefined,
				)
				.filter((t) => t !== undefined),
		[telemetryList],
	);

	const speedChartData = useMemo(
		() =>
			telemetryList.flatMap((t) => {
				const date = new Date(t.timestamp);
				return [
					{
						name: date.toLocaleString(),
						label: STATE_ICONS[t.state],
						accuracy: t.accuracy?.toFixed(2),
						speed: toKMH(t.speed ?? 0).toFixed(2),
					},
				];
			}),
		[telemetryList],
	);

	return (
		<main className="bg-gray-100 min-h-screen">
			<header className="p-4 bg-white shadow-sm border-b border-gray-200 sticky top-0 z-9999 w-full select-none cursor-default">
				<h1 className="font-bold">TrainLCD THQ</h1>
			</header>

			<section className="px-4 pb-4 mt-4">
				<h3 className="text-md font-semibold mb-2">Visualize</h3>
				{latestTelemetry ? (
					<div className="mt-2 flex gap-4">
						<div className="h-96 w-1/2">
							<CurrentLocationMap
								locations={locations}
								state={latestTelemetry.state}
								bearing={latestTelemetryBearing}
								badAccuracy={badAccuracy}
								device={latestTelemetry.device}
							/>
						</div>
						<div className="h-96 w-1/2">
							<SpeedChart data={speedChartData} />
						</div>
					</div>
				) : (
					<div className="w-1/2">
						<p className="text-gray-500">No location data available.</p>
					</div>
				)}

				<div className="mt-4">
					<h3 className="text-md font-semibold mb-2">Moving Log</h3>
					{movingLogs.length ? (
						<MovingLogTable movingLogs={movingLogs} />
					) : (
						<p className="text-gray-500">No moving log data available.</p>
					)}
				</div>

				<div className="mt-4">
					<h3 className="text-md font-semibold mb-2">Error</h3>
					{error ? (
						<div className="mt-2">
							<p>Error Type: {error.type}</p>
							<p>Raw Data: {JSON.stringify(error.raw)}</p>
						</div>
					) : (
						<p className="text-gray-500">No error data available.</p>
					)}
				</div>

				<div className="mt-4">
					<h3 className="text-md font-semibold mb-2">Logs</h3>
					{sortedConsoleLogs.length ? (
						<ConsoleLogTable logs={sortedConsoleLogs} />
					) : (
						<p className="text-gray-500">No log data available.</p>
					)}
				</div>
			</section>
		</main>
	);
}

export default memo(App);
