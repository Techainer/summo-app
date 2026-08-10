import { People } from "../components/People";
import { useEngine } from "../lib/engine-context";

export function PeopleScreen() {
  const { people } = useEngine();
  return <People client={people} />;
}
