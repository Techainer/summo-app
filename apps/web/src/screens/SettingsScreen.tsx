import { Settings } from "../components/Settings";
import { useEngine } from "../lib/engine-context";

export function SettingsScreen() {
  const { handshake } = useEngine();
  return <Settings handshake={handshake} />;
}
