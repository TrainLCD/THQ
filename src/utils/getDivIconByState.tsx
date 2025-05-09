import { BoltIcon, ArrowRightIcon, PauseIcon } from "@heroicons/react/24/solid";
import type { MovingState } from "~/domain/commands";

export const getStateBadgeIcon = (state: MovingState) => {
	switch (state) {
		case "passing":
			return <BoltIcon className="w-3 h-3" />;
		case "approaching":
			return <ArrowRightIcon className="w-3 h-3" />;
		case "arrived":
			return <PauseIcon className="w-3 h-3" />;
		default:
			return null;
	}
};
