import {
	CartesianGrid,
	Legend,
	Line,
	LineChart,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";

export const SpeedChart = ({
	data,
}: { data: { name: string; speed: number; gForce: number }[] }) => (
	<ResponsiveContainer width="100%" height="100%">
		<LineChart
			data={data}
			width={500}
			height={300}
			className="bg-white rounded-md"
		>
			<CartesianGrid strokeDasharray="3 3" />
			<XAxis dataKey="timestamp" />

			{/* Primary axis for speed & gForce */}
			<YAxis yAxisId="left" domain={[0, 2]} />
			{/* Secondary axis for accuracy */}
			<YAxis yAxisId="right" orientation="right" domain={[0, 30]} />

			<Tooltip />
			<Legend />

			<Line
				yAxisId="left"
				dataKey="speed"
				stroke="#8884d8"
				name="Speed (m/s)"
			/>
			<Line
				yAxisId="left"
				dataKey="gForce"
				stroke="#82ca9d"
				name="G-Force (g)"
			/>
			<Line
				yAxisId="right"
				dataKey="accuracy"
				stroke="#ff7300"
				name="Accuracy (m)"
			/>
		</LineChart>
	</ResponsiveContainer>
);
