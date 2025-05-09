import { memo } from "react";
import type { LocationData } from "~/domain/commands";
import { STATE_ICONS } from "~/domain/emoji";
import { BAD_ACCURACY_THRESHOLD } from "~/domain/threshold";
import { toKMH } from "~/utils/unit";

export const MovingLogTable = memo(
	({ movingLogs }: { movingLogs: LocationData[] }) => {
		return (
			<div className="overflow-auto max-h-96 overscroll-none">
				<table className="bg-white w-full border-spacing-2 border border-gray-200 rounded-md">
					<thead className="sticky top-0 bg-white border borer-gray-200">
						<tr>
							<th className="p-2 border border-gray-200 w-16">state</th>
							<th className="p-2 border border-gray-200">timestamp</th>
							<th className="p-2 border border-gray-200">coordinates</th>
							<th className="p-2 border border-gray-200">speed</th>
							<th className="p-2 border border-gray-200">accuracy</th>
							<th className="p-2 border border-gray-200">device</th>
						</tr>
					</thead>
					<tbody>
						{movingLogs.map((t) => (
							<tr key={t.id}>
								<td className="p-2 border border-gray-200 w-16 text-center">
									{STATE_ICONS[t.state]}
								</td>
								<td className="p-2 border border-gray-200">
									{new Date(t.timestamp).toLocaleString()}
								</td>
								<td className="p-2 border border-gray-200">
									{t.lon?.toFixed(5)}, {t.lat?.toFixed(5)}
								</td>
								<td className="p-2 border border-gray-200">
									{(t.speed ?? 0)?.toFixed(2)}m/s (
									{toKMH(t.speed ?? 0).toFixed(2)}
									km/h)
								</td>
								{(t?.accuracy ?? 0) > BAD_ACCURACY_THRESHOLD ? (
									<td className="p-2 border border-gray-200 text-red-600 font-bold">
										{t.accuracy?.toFixed(2)}m
									</td>
								) : (
									<td className="p-2 border border-gray-200">
										{t.accuracy?.toFixed(2)}m
									</td>
								)}
								<td className="p-2 border border-gray-200">{t.device}</td>
							</tr>
						))}
					</tbody>
				</table>
			</div>
		);
	},
);
