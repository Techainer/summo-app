import { useNavigate, useSearch } from "@tanstack/react-router";

import { Settings } from "../components/Settings";
import type { SectionId } from "../lib/settings";
import { useEngine } from "../lib/engine-context";

/**
 * Which settings section is open lives in the URL.
 *
 * So that "open Settings → Storage" can be a link: the onboarding checklist, a nudge about disk
 * space and the ⌘K palette all want to send somebody to a particular setting rather than to the top
 * of a long screen — and so a reload comes back to the section that was open.
 */
export function SettingsScreen() {
  const { handshake } = useEngine();
  const navigate = useNavigate();
  const { section } = useSearch({ from: "/settings" });

  return (
    <Settings
      handshake={handshake}
      section={section}
      onSection={(next: SectionId) =>
        // `replace`, because moving along a rail is not six steps of history to walk back out of.
        void navigate({ to: "/settings", search: { section: next }, replace: true })
      }
    />
  );
}
