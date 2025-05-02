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
}: { data: { name: string; speed: number }[] }) => (
	<ResponsiveContainer width="100%" height="100%">
		<LineChart
			data={data}
			width={500}
			height={300}
			className="bg-white rounded-md"
		>
			<CartesianGrid strokeDasharray="3 3" />
			<XAxis dataKey="timestamp" />

			{/* Primary axis for speed */}
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
				yAxisId="right"
				dataKey="accuracy"
				stroke="#ff7300"
				name="Accuracy (m)"
			/>
		</LineChart>
	</ResponsiveContainer>
);
