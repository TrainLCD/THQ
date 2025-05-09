import { memo } from "react";
import type { LogData } from "~/domain/commands";
import { LOG_LEVEL_ICONS } from "~/domain/emoji";

export const ConsoleLogTable = memo(({ logs }: { logs: LogData[] }) => {
	return (
		<div className="overflow-auto max-h-96 overscroll-none">
			<table className="bg-white w-full border-spacing-2 border border-gray-200 rounded-md">
				<thead className="sticky top-0 bg-white border-b-1 borer-b-gray-200">
					<tr>
						<th className="p-2 border border-gray-200 w-16">level</th>
						<th className="p-2 border border-gray-200">timestamp</th>
						<th className="p-2 border border-gray-200">message</th>
						<th className="p-2 border border-gray-200">device</th>
					</tr>
				</thead>
				<tbody>
					{logs.map((l) => (
						<tr key={l.id}>
							<td className="p-2 border border-gray-200 w-16 text-center">
								{LOG_LEVEL_ICONS[l.level]}
							</td>
							<td className="p-2 border border-gray-200">
								{new Date(l.timestamp).toLocaleString()}
							</td>
							<td className="p-2 border border-gray-200">{l.message}</td>
							<td className="p-2 border border-gray-200">{l.device}</td>
						</tr>
					))}
				</tbody>
			</table>
		</div>
	);
});
