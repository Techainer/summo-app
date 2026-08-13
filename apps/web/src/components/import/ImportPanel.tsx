import { useNavigate } from "@tanstack/react-router";
import { FileAudio } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useT } from "../../i18n/context";
import { useErrorText } from "../../lib/errors";
import { GENTLE, METER, listItem } from "../../lib/motion";
import { useEngine } from "../../lib/engine-context";
import {
  ImportClient,
  POLL_MS,
  baseName,
  describe,
  isFinished,
  percent,
  pickFile,
  type Job,
} from "../../lib/imports";
import { Button } from "../ui";
import { useRefresh } from "../../lib/use-load";

/**
 * The other way a meeting gets into Summo: a file that already exists.
 *
 * Polling starts when something is running and stops when nothing is, rather than running for the
 * life of the screen — an idle poll every two seconds is a wakeup every two seconds, and this
 * screen is often left open.
 *
 * There is no drag-and-drop, and the panel is shaped like a drop target anyway. That is not a
 * bluff — it is the shape people read as "put a file here", and the button inside it does exactly
 * that. What a browser drop *cannot* do is give a path: it gives a `File` whose bytes live in the
 * page, and the daemon needs a path to hand ffmpeg. Accepting one would mean reading a
 * two-gigabyte video into the webview and posting it back to a process on the same disk.
 *
 * The desktop shell could do it properly — Tauri's drop event carries real paths — and that is the
 * version worth building. It is not built here because it cannot be tested from a browser, and a
 * drop handler that silently does nothing in the build most people run is worse than a button.
 */
export function ImportPanel() {
  const { handshake } = useEngine();
  const errorText = useErrorText();
  const client = useMemo(() => new ImportClient(handshake), [handshake]);
  const navigate = useNavigate();
  const t = useT();

  const [jobs, setJobs] = useState<Job[]>([]);
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setJobs(await client.list());
    } catch {
      // A daemon that is briefly busy is not a reason to blank the list the user is reading.
    }
  }, [client]);

  useRefresh(refresh);

  const busy = jobs.some((job) => !isFinished(job));
  useEffect(() => {
    if (!busy) return undefined;
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [busy, refresh]);

  // A `Described` is either a key to translate or the daemon's own text.
  const say = (described: ReturnType<typeof describe>) =>
    "text" in described ? described.text : t(described.key, described.values);

  const submit = async (chosen: string) => {
    const trimmed = chosen.trim();
    if (!trimmed) return;
    setStarting(true);
    setError(null);
    try {
      const job = await client.start(trimmed);
      setJobs((current) => [job, ...current]);
      setPath("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setStarting(false);
    }
  };

  const browse = async () => {
    const chosen = await pickFile(t("import.file_filter"));
    // Outside Tauri there is no dialog; the text field stays the way in, so say so once rather
    // than failing silently on a button that appears to do nothing.
    if (chosen === null) {
      setError(t("import.no_dialog"));
      return;
    }
    await submit(chosen);
  };

  return (
    <div className="mx-auto mt-10 w-full max-w-xl px-4">
      {/* A dashed panel, because that is the shape people read as "a file goes here" — and the
          button inside it is what puts one there. The bare text field this replaced said nothing
          about what the screen wanted, what it accepted, or where the file would go. */}
      <div className="border-line-strong bg-bg-soft/40 hover:border-fg-faint flex flex-col items-center gap-3 rounded-[var(--radius-panel)] border border-dashed px-6 py-10 text-center transition-colors">
        <span className="bg-bg-soft ring-line grid size-12 place-items-center rounded-full ring-1">
          <FileAudio aria-hidden="true" className="text-fg-faint size-5 stroke-[1.5]" />
        </span>
        <div>
          <p className="font-medium">{t("import.title")}</p>
          <p className="text-fg-dim text-meta mx-auto mt-1 max-w-sm leading-relaxed">
            {t("import.hint")}
          </p>
        </div>
        <Button onClick={() => void browse()}>{t("import.browse")}</Button>
        {/* The formats, said once and plainly. Finding out by being refused is the worst way. */}
        <p className="text-fg-faint text-micro font-mono">{t("import.formats")}</p>
      </div>

      <div className="mt-3 flex items-center gap-2">
        <span className="text-fg-faint text-micro shrink-0">{t("import.or_path")}</span>
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit(path);
          }}
          placeholder={t("import.path_placeholder")}
          aria-label={t("import.path_label")}
          className="border-line bg-bg-soft focus:border-accent text-meta min-w-0 flex-1 rounded-lg border px-3 py-1.5 outline-none"
        />
        <Button size="sm" onClick={() => void submit(path)} disabled={!path.trim() || starting}>
          {t("import.submit")}
        </Button>
      </div>

      {error && (
        <p role="alert" className="text-danger mt-2 text-sm">
          {error}
        </p>
      )}

      {/* No empty state for the job list. The panel above already says what this screen wants and
          what happens to the file; a second block below repeating it is two paragraphs of the same
          sentence stacked, which is what it looked like when it was there. */}
      <ul className="mt-6 space-y-2">
        <AnimatePresence initial={false}>
          {jobs.map((job) => {
            const pct = percent(job);
            return (
              <motion.li
                key={job.id}
                variants={listItem}
                initial="hidden"
                animate="shown"
                exit="gone"
                transition={GENTLE}
                className="border-line bg-bg-soft rounded-xl border p-3"
              >
                <div className="flex items-baseline justify-between gap-3">
                  <span className="min-w-0 truncate text-sm font-medium">
                    {job.title || baseName(job.source)}
                  </span>
                  <span
                    className={
                      job.state === "failed" ? "text-danger text-meta" : "text-fg-dim text-meta"
                    }
                  >
                    {say(describe(job))}
                  </span>
                </div>

                {!isFinished(job) && (
                  <div
                    className="bg-line mt-2 h-1 overflow-hidden rounded-full"
                    role="progressbar"
                    aria-valuenow={pct ?? undefined}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-label={t("import.progress_label", {
                      title: job.title,
                    })}
                  >
                    {/* Length unknown: an indeterminate sweep, because a bar frozen at 0% is the
                        one thing a five-minute job must not look like. */}
                    <motion.div
                      className="bg-accent h-full"
                      animate={pct === null ? { x: ["-100%", "100%"] } : { width: `${pct}%` }}
                      transition={
                        pct === null ? { repeat: Infinity, duration: 1.2, ease: "linear" } : METER
                      }
                      style={pct === null ? { width: "40%" } : undefined}
                    />
                  </div>
                )}

                {job.state === "done" && job.meeting && (
                  <button
                    type="button"
                    onClick={() =>
                      void navigate({
                        to: "/meetings/$meetingId",
                        params: { meetingId: job.meeting as string },
                      })
                    }
                    className="text-accent text-meta mt-2 font-medium hover:underline"
                  >
                    {t("import.open_meeting")}
                  </button>
                )}
              </motion.li>
            );
          })}
        </AnimatePresence>
      </ul>
    </div>
  );
}
