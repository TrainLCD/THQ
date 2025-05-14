import { type FirebaseApp, initializeApp } from "firebase/app";
import { getAuth, signInAnonymously, type Unsubscribe } from "firebase/auth";
import { collection, getFirestore, onSnapshot } from "firebase/firestore";
import { useAtom } from "jotai";
import uniqBy from "lodash/uniqBy";
import { useCallback, useEffect, useRef, useState } from "react";
import { telemetryListAtom } from "~/atoms/telemetryItem";
import {
	ErrorData,
	LocationData,
	type LogData,
	registerTelemetryListener,
} from "~/domain/commands";
import { isLocalServerEnabledAsync } from "~/utils/server";

export const useTelemetry = () => {
	const [isLocalServerAvailable, setIsLocalServerAvailable] = useState<
		boolean | null
	>(null);
	const [error, setError] = useState<ErrorData | null>(null);
	const [consoleLogs, setConsoleLogs] = useState<LogData[]>([]);
	const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

	const firebaseAppRef = useRef<FirebaseApp | null>(null);

	const handleLocationUpdate = useCallback(
		(data: LocationData) => {
			setTelemetryList((prev) =>
				uniqBy(
					[
						...prev.slice(-9999), // 最大10,000件まで保持
						{ ...data },
					],
					"id",
				),
			);
		},
		[setTelemetryList],
	);

	useEffect(() => {
		const updateServerAvailabilityAsync = async () => {
			setIsLocalServerAvailable(await isLocalServerEnabledAsync());
		};
		updateServerAvailabilityAsync();
	}, []);

	useEffect(() => {
		if (!isLocalServerAvailable) {
			return;
		}

		registerTelemetryListener({
			onLocationUpdate: handleLocationUpdate,
			onError: (err) => setError(err),
			onLog: (log) => {
				setConsoleLogs((prev) => uniqBy([...prev, log], "id"));
			},
		});
	}, [handleLocationUpdate, isLocalServerAvailable]);

	useEffect(() => {
		if (isLocalServerAvailable) {
			return;
		}

		let unsubLocations: Unsubscribe | null = null;

		const firebaseConfig = {
			apiKey: import.meta.env.VITE_FIR_API_KEY,
			authDomain: import.meta.env.VITE_FIR_AUTH_DOMAIN,
			databaseURL: import.meta.env.VITE_FIR_DATABASE_URL,
			projectId: import.meta.env.VITE_FIR_PROJECT_ID,
			storageBucket: import.meta.env.VITE_FIR_STORAGE_BUCKET,
			messagingSenderId: import.meta.env.VITE_FIR_MESSAGING_SENDER_ID,
			appId: import.meta.env.VITE_FIR_APP_ID,
			measurementId: import.meta.env.VITE_FIR_MEASUREMENT_ID,
		};

		const setupFirebaseAsync = async () => {
			const app = initializeApp(firebaseConfig);
			firebaseAppRef.current = app;

			const auth = getAuth(app);

			try {
				await signInAnonymously(auth);
			} catch (err) {
				const errData = ErrorData.parse({ type: "unknown", raw: err });
				setError(errData);
			}

			const db = getFirestore();
			unsubLocations = onSnapshot(
				collection(db, "telemetryLocations"),
				(snapshot) => {
					for (const change of snapshot.docChanges()) {
						const docData = change.doc.data();
						const locationData = LocationData.safeParse({
							id: change.doc.id,
							lat: docData.latitude,
							lon: docData.longitude,
							timestamp: docData.timestamp?.toDate().getTime(),
						});

						if (locationData.error) {
							return;
						}

						switch (change.type) {
							case "added":
								setTelemetryList((prev) =>
									uniqBy(
										[
											...prev.slice(-9999), // 最大10,000件まで保持
											locationData.data,
										],
										"id",
									),
								);
								break;
							case "modified":
								break;
							case "removed":
								break;
						}
					}
				},
			);
		};

		setupFirebaseAsync();

		return () => {
			unsubLocations?.();
		};
	}, [isLocalServerAvailable, setTelemetryList]);

	return {
		telemetryList,
		error,
		consoleLogs,
		isLocalServerAvailable,
	};
};
