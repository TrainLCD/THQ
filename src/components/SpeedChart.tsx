import {
	CartesianGrid,
	Label,
	Legend,
	Line,
	LineChart,
	ReferenceLine,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";

type SpeedChartData = { name: string; speed: string };

export const SpeedChart = ({ data }: { data: SpeedChartData[] }) => (
	<ResponsiveContainer width="100%" height="100%">
		<LineChart
			data={data.slice(-10)}
			width={500}
			height={300}
			className="bg-white rounded-md"
			margin={{ top: 30, right: 30, left: 30, bottom: 10 }}
		>
			<CartesianGrid strokeDasharray="3 3" />
			<XAxis dataKey="label" />

			{/* Primary axis for speed */}
			<YAxis yAxisId="left" domain={[0, 130]}>
				<Label value="Speed (km/h)" position="top" />
			</YAxis>
			{/* Secondary axis for accuracy */}
			<YAxis yAxisId="right" orientation="right" domain={[0, 130]}>
				<Label value="Accuracy (m)" position="top" />
			</YAxis>

			<ReferenceLine
				y={100}
				yAxisId="right"
				label="Low Accuracy"
				stroke="red"
				strokeDasharray="3 3"
			/>

			<Tooltip />
			<Legend />

			<Line
				yAxisId="left"
				dataKey="speed"
				stroke="#8884d8"
				name="Speed (km/h)"
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
