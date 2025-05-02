import type { LatLngTuple } from "leaflet";
import "leaflet/dist/leaflet.css";
import { useEffect } from "react";
import {
	MapContainer,
	Marker,
	Polyline,
	Popup,
	TileLayer,
	useMap,
} from "react-leaflet";

const MapFollower = ({ center }: { center: LatLngTuple }) => {
	const map = useMap();

	useEffect(() => {
		map.setView(center);
	}, [center, map.setView]);

	return null;
};

export const CurrentLocationMap = ({
	locations,
}: { locations: LatLngTuple[] }) => {
	const [lat, lon] = locations[locations.length - 1];

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
			<Marker position={[lat, lon]}>
				<Popup>
					現在地
					<br />
					{lat}, {lon}
				</Popup>
			</Marker>
			<Polyline positions={locations} color="blue" />
			<MapFollower center={[lat, lon]} />
		</MapContainer>
	);
};
