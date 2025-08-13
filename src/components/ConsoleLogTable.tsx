import { memo } from "react";
import type { LogData } from "~/domain/commands";
import { LOG_LEVEL_ICONS } from "~/domain/emoji";

export const ConsoleLogTable = memo(({ logs }: { logs: LogData[] }) => {
  return (
    <div className="overflow-y-scroll max-h-96 overscroll-none border border-gray-200 dark:border-white/15 rounded-md">
      <table className="bg-white dark:bg-white/5 w-full">
        <thead className="sticky shadow top-0 bg-white dark:bg-black border-b-1 borer-b-gray-200 dark:border-b-white/15">
          <tr>
            <th className="p-2 border border-gray-200 dark:border-white/15 w-16">
              level
            </th>
            <th className="p-2 border border-gray-200 dark:border-white/15">
              timestamp
            </th>
            <th className="p-2 border border-gray-200 dark:border-white/15">
              message
            </th>
            <th className="p-2 border border-gray-200 dark:border-white/15">
              device
            </th>
          </tr>
        </thead>
        <tbody>
          {logs.map((l) => (
            <tr key={l.id}>
              <td className="p-2 border border-gray-200 dark:border-white/15 w-16 text-center">
                {LOG_LEVEL_ICONS[l.level]}
              </td>
              <td className="p-2 border border-gray-200 dark:border-white/15">
                {new Date(l.timestamp).toLocaleString()}
              </td>
              <td className="p-2 border border-gray-200 dark:border-white/15">
                {l.message}
              </td>
              <td className="p-2 border border-gray-200 dark:border-white/15">
                {l.device}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
});
