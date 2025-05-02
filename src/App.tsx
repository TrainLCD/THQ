import { useTelemetry } from "./hooks/useTelemetry";
import { CurrentLocationMap } from "./components/CurrentLocationMap";
import { useMemo } from "react";

function App() {
	const { telemetryList, error } = useTelemetry();

	const latestTelemetry = useMemo(
		() => telemetryList[telemetryList.length - 1],
		[telemetryList],
	);

	return (
		<main className="bg-gray-100 min-h-screen">
			<header className="p-4 bg-white shadow-sm border-b border-gray-200 sticky top-0 z-9999 w-full select-none cursor-default">
				<h1 className="font-bold">TrainLCD THQ</h1>
			</header>

			<section className="px-4 pb-4 mt-4">
				<h3 className="text-md font-semibold">Visualize</h3>
				<div className="mt-2">
					{latestTelemetry ? (
						<div className="h-96 w-full mt-2">
							<CurrentLocationMap
								location={[latestTelemetry.lat, latestTelemetry.lon]}
							/>
						</div>
					) : (
						<p className="text-gray-500">No location data available.</p>
					)}
				</div>

				<div className="mt-4">
					<h3 className="text-md font-semibold">Error</h3>
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
					<h3 className="text-md font-semibold">Logs</h3>
					<div className="mt-2">
						<table className="bg-white w-full border-spacing-2 border border-gray-200 rounded-lg">
							<thead>
								<tr>
									<th className="p-2 border border-gray-200">timestamp</th>
									<th className="p-2 border border-gray-200">lat</th>
									<th className="p-2 border border-gray-200">lon</th>
									<th className="p-2 border border-gray-200">speed(m/s)</th>
									<th className="p-2 border border-gray-200">gForce(g)</th>
									<th className="p-2 border border-gray-200">accuracy(m)</th>
								</tr>
							</thead>
							<tbody>
								{telemetryList.map((t) => (
									<tr key={t.timestamp}>
										<td className="p-2 border border-gray-200">
											{new Date(latestTelemetry.timestamp).toLocaleString()}
										</td>
										<td className="p-2 border border-gray-200">
											{t.lat.toFixed(5)}
										</td>
										<td className="p-2 border border-gray-200">
											{t.lon.toFixed(5)}
										</td>
										<td className="p-2 border border-gray-200">
											{t.speed.toFixed(2)}
										</td>
										<td className="p-2 border border-gray-200">
											{t.gForce.toFixed(3)}
										</td>
										{t.accuracy > 100 ? (
											<td className="p-2 border border-gray-200 text-red-600 font-bold">
												{t.accuracy.toFixed(2)}
											</td>
										) : (
											<td className="p-2 border border-gray-200">
												{t.accuracy.toFixed(2)}
											</td>
										)}
									</tr>
								))}
							</tbody>
						</table>
					</div>
				</div>
			</section>
		</main>
	);
}

export default App;
