import { atom } from "jotai";
import type { LocationData } from "../domain/commands";

export const telemetryListAtom = atom<LocationData[]>([]);
