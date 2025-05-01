import { useTelemetry } from "./hooks/useTelemetry";

function App() {
	const { location, error } = useTelemetry();

	return (
		<main>
			<header className="p-4 bg-white shadow-sm border-b border-gray-200">
				<h1 className="font-bold">TrainLCD THQ</h1>
			</header>

			<section className="p-4">
				<h2 className="text-lg font-semibold">Telemetry</h2>
				<div className="mt-4">
					<h3 className="text-md font-semibold">Location</h3>
					{location ? (
						<div className="mt-2">
							<p>Latitude: {location.lat}</p>
							<p>Longitude: {location.lon}</p>
							<p>Accuracy: {location.accuracy} m</p>
							<p>Speed: {location.speed} m/s</p>
							<p>G-Force: {location.gForce} g</p>
							<p>Timestamp: {new Date(location.timestamp).toLocaleString()}</p>
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
			</section>
		</main>
	);
}

export default App;
