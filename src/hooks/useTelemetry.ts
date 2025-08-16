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
  const [isLocalServerEnabled, setisLocalServerEnabled] = useState(false);
  const [error, setError] = useState<ErrorData | null>(null);
  const [consoleLogs, setConsoleLogs] = useState<LogData[]>([]);
  const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

  const handleReceivedLocationUpdate = useCallback(
    (data: LocationData) =>
      setTelemetryList((prev) => [...prev, data].slice(-1000)),
    [setTelemetryList]
  );
  const handleReceivedError = useCallback(
    (err: ErrorData) => setError(err),
    []
  );
  const handleReceivedLog = useCallback(
    (data: LogData) => setConsoleLogs((prev) => [...prev, data].slice(-1000)),
    []
  );

  useEffect(() => {
    let disposed = false;
    const updateServerAvailabilityAsync = async () => {
      try {
        const avail = await isLocalServerEnabledAsync();
        if (!disposed) setisLocalServerEnabled(avail);
      } catch (e) {
        console.error("Failed to check local server availability", e);
        if (!disposed)
          setError({
            type: "unknown",
            reason: e instanceof Error ? e.message : String(e),
          });
      }
    };
    updateServerAvailabilityAsync();
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    registerTelemetryListener({
      onLocationUpdate: handleReceivedLocationUpdate,
      onError: handleReceivedError,
      onLog: handleReceivedLog,
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
        if (!disposed)
          setError({
            type: "unknown",
            reason: e instanceof Error ? e.message : String(e),
          });
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [handleReceivedLocationUpdate, handleReceivedError, handleReceivedLog]);

  return {
    telemetryList,
    error,
    consoleLogs,
    isLocalServerEnabled,
  };
};
