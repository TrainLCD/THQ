import { getMatches } from "@tauri-apps/plugin-cli";
import { CLI_ARGS } from "~/constants/cli";

export const isLocalServerEnabledAsync = async () => {
	const matches = await getMatches();
	const isLocalServerEnabled =
		!!matches.args[CLI_ARGS.LOCAL_SERVER_ENABLED]?.value;
	return isLocalServerEnabled;
};
