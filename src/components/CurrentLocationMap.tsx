import "leaflet/dist/leaflet.css";
import { useEffect } from "react";
import { MapContainer, Marker, Popup, TileLayer, useMap } from "react-leaflet";

const MapFollower = ({ center }: { center: [number, number] }) => {
	const map = useMap();

	useEffect(() => {
		map.setView(center);
	}, [center, map.setView]);

	return null;
};

export const CurrentLocationMap = ({
	location: [lat, lon],
}: { location: [number, number] }) => {
	return (
		<MapContainer
			center={[lat, lon]}
			zoom={15}
			className="size-full"
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
			<MapFollower center={[lat, lon]} />
		</MapContainer>
	);
};
