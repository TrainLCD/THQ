import type { UnlistenFn } from "@tauri-apps/api/event";
import { useAtom } from "jotai";
import { useCallback, useEffect, useState } from "react";
import { telemetryListAtom } from "~/atoms/telemetryItem";
import {
  type ErrorData,
  type LocationData,
  type LogData,
  registerTelemetryListener,
} from "~/domain/commands";
import { isLocalServerEnabledAsync } from "~/utils/server";

export const useTelemetry = () => {
  const [isLocalServerAvailable, setIsLocalServerAvailable] = useState(false);
  const [error, setError] = useState<ErrorData | null>(null);
  const [consoleLogs, setConsoleLogs] = useState<LogData[]>([]);
  const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

  const handleLocationUpdate = useCallback(
    (data: LocationData) => {
      setTelemetryList((prev) => {
        const filtered = prev.filter((x) => x.id !== data.id);
        // 最大1,000件に制限してメモリ使用量を管理（重複は最新を優先）
        return [...filtered, { ...data }].slice(-1000);
      });
    },
    [setTelemetryList]
  );

  useEffect(() => {
    const updateServerAvailabilityAsync = async () => {
      try {
        setIsLocalServerAvailable(await isLocalServerEnabledAsync());
      } catch (e) {
        console.error("Failed to check local server availability", e);
        setError({ type: "unknown", raw: e });
      }
    };
    updateServerAvailabilityAsync();
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    registerTelemetryListener({
      onLocationUpdate: handleLocationUpdate,
      onError: (err) => setError(err),
      onLog: (log) =>
        setConsoleLogs((prev) => {
          const filtered = prev.filter((x) => x.id !== log.id);
          // コンソールログも最大1,000件に制限（重複は最新を優先）
          return [...filtered, log].slice(-1000);
        }),
    })
      .then((fn) => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((e) => {
        console.error("Failed to register telemetry listener", e);
        setError({ type: "unknown", raw: e });
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [handleLocationUpdate]);

  return {
    telemetryList,
    error,
    consoleLogs,
    isLocalServerAvailable,
  };
};
