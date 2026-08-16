import { useState } from "react";

import { Permissions } from "../onboarding/Permissions";
import { SpokenLanguage } from "../record/SpokenLanguage";
import { load as loadCapture, save as saveCapture } from "../../lib/capture";
import { useT } from "../../i18n/context";

/**
 * Everything that decides what a recording will be like before it starts: whether the microphone is
 * allowed, and what language to expect.
 *
 * Together because they fail together. Somebody arriving here after a recording that produced
 * nothing is looking for one of these two, and having them on separate screens meant checking the
 * wrong one first.
 */
export function Recording() {
  const t = useT();
  const [spoken, setSpoken] = useState(() => loadCapture().spoken);

  return (
    <div data-testid="settings-recording">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("settings.recording_hint")}</p>

      {/* First, because it is the one that stops a recording outright. */}
      <Permissions />

      {/* The same control the record bar carries, because it is the same decision — but it belongs
          here too: the record bar is where you change it for *this* meeting, and this is where you
          change what every meeting starts from. Somebody who records in one language and has to
          reselect it every time is being asked a question that was already answered.

          `SpokenLanguage` writes through to the daemon itself; this only keeps the browser's own
          copy in step, so the record bar opens on the same answer. */}
      <section className="border-line bg-bg-raised mt-6 rounded-2xl border p-5">
        <h3 className="font-medium">{t("settings.spoken_heading")}</h3>
        <p className="text-fg-dim text-meta mt-1 mb-3">{t("settings.spoken_hint")}</p>
        <SpokenLanguage
          value={spoken}
          onChange={(code) => {
            setSpoken(code);
            saveCapture({ ...loadCapture(), spoken: code });
          }}
        />
      </section>
    </div>
  );
}
