import { createContext, useContext } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { Config } from "../../lib/config";

export interface SettingsCore {
  config: Config;
  setConfig: Dispatch<SetStateAction<Config>>;
  update: (path: string, value: string | number) => void;
}

export const SettingsCoreContext = createContext<SettingsCore | null>(null);

export function useSettingsCore(): SettingsCore {
  const core = useContext(SettingsCoreContext);
  if (!core) {
    throw new Error("useSettingsCore must be used inside <Settings>");
  }
  return core;
}
