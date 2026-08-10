import { useNavigate } from "@tanstack/react-router";

import { Library } from "../components/Library";
import { useEngine } from "../lib/engine-context";

export function LibraryScreen() {
  const { library, start } = useEngine();
  const navigate = useNavigate();
  return (
    <Library
      client={library}
      onRecord={() => {
        void navigate({ to: "/" });
        void start();
      }}
    />
  );
}
