import type { LatLngTuple } from "leaflet";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { memo, useEffect, useMemo, useRef } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
	MapContainer,
	Marker,
	Polyline,
	Popup,
	TileLayer,
	useMap,
} from "react-leaflet";
import type { MovingState } from "~/domain/commands";
import {
	CaretRight,
	CaretDoubleRight,
	CaretLineRight,
	House,
} from "@phosphor-icons/react";
import { STATE_ICONS } from "~/domain/emoji";
import { STATE_LABELS } from "~/domain/text";

const MapFollower = ({ center }: { center: LatLngTuple }) => {
	const map = useMap();

	useEffect(() => {
		map.setView(center);
	}, [center, map.setView]);

	return null;
};

const stateIcons = {
	arrived: House,
	approaching: CaretLineRight,
	passing: CaretDoubleRight,
	moving: CaretRight,
};

export const CurrentLocationMap = memo(
	({
		locations,
		state,
		bearing,
		badAccuracy,
		device,
	}: {
		locations: LatLngTuple[];
		state: MovingState;
		bearing: number;
		badAccuracy: boolean;
		device: string;
	}) => {
		const markerRef = useRef<L.Marker>(null);

		const [lat, lon] = useMemo(
			() => locations[locations.length - 1],
			[locations],
		);

		const iconForState = useMemo(() => {
			const IconComponent = stateIcons[state];
			if (!IconComponent) {
				return;
			}

			const html = renderToStaticMarkup(
				<div className={"relative w-8 h-8"}>
					<div
						className={
							badAccuracy
								? "absolute w-8 h-8 bg-orange-400 rounded-full border-2 border-white flex items-center justify-center shadow-sm shadow-orange-400"
								: "absolute w-8 h-8 bg-blue-500 rounded-full border-2 border-white flex items-center justify-center shadow-sm shadow-blue-500"
						}
					>
						<div className="transition-transform duration-300 icon-inner fade-in opacity-0">
							<IconComponent
								key={`${state}-${badAccuracy}`}
								weight="bold"
								className="w-4 h-4 text-white"
							/>
						</div>
					</div>
				</div>,
			);
			return L.divIcon({
				html,
				className: "", // 不要な leaflet-icon クラスを無効化
				iconSize: [24, 24],
				iconAnchor: [16, 18],
			});
		}, [state, badAccuracy]);

		useEffect(() => {
			if (markerRef.current && state !== "arrived") {
				const el = markerRef.current.getElement();
				const icon = el?.querySelector(".icon-inner") as HTMLElement | null;
				if (icon) {
					const deg = (bearing + 90) % 360;
					icon.style.transform = `rotate(${deg}deg)`;
				}
			}
		}, [bearing, state]);

		return (
			<MapContainer
				center={[lat, lon]}
				zoom={15}
				className="size-full rounded-md"
				scrollWheelZoom={false}
			>
				<TileLayer
					attribution='&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors'
					url="https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png"
				/>
				<Marker position={[lat, lon]} icon={iconForState} ref={markerRef}>
					<Popup>
						<div className="mb-1">
							<p className="font-bold">📱&nbsp;{device}</p>
							<p className="font-bold">
								{STATE_ICONS[state]}&nbsp;{STATE_LABELS[state]}
							</p>
							{badAccuracy && (
								<p className="text-orange-500 font-bold">
									📍&nbsp;低い位置情報精度
								</p>
							)}
						</div>
						<p>
							現在地:
							{lat.toFixed(5)}, {lon.toFixed(5)}
						</p>
					</Popup>
				</Marker>
				<Polyline positions={locations} color="#2b7fff" />
				<MapFollower center={[lat, lon]} />
			</MapContainer>
		);
	},
);
