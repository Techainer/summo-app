import { useNavigate } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useT } from "../../i18n/context";
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

/**
 * The other way a meeting gets into Summo: a file that already exists.
 *
 * Polling starts when something is running and stops when nothing is, rather than running for the
 * life of the screen — an idle poll every two seconds is a wakeup every two seconds, and this
 * screen is often left open.
 *
 * There is no drag-and-drop. A browser drop gives a `File` whose bytes live in the page, and the
 * daemon needs a path to hand ffmpeg; accepting a drop would mean reading a two-gigabyte video into
 * the webview and posting it back to a process on the same disk. The file dialog returns the path
 * directly, so that is the affordance.
 */
export function ImportPanel() {
  const { handshake } = useEngine();
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

  useEffect(() => {
    void refresh();
  }, [refresh]);

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
      setError(e instanceof Error ? e.message : String(e));
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
      <h2 className="text-lg font-medium">{t("import.title")}</h2>
      <p className="mt-1 text-sm text-fg-dim">
        {t("import.hint")}
      </p>

      <div className="mt-4 flex gap-2">
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit(path);
          }}
          placeholder={t("import.path_placeholder")}
          aria-label={t("import.path_label")}
          className="min-w-0 flex-1 rounded-xl border border-line bg-bg-soft px-3 py-2 text-sm outline-none focus:border-accent"
        />
        <Button onClick={() => void browse()} variant="ghost">
          {t("import.browse")}
        </Button>
        <Button onClick={() => void submit(path)} disabled={!path.trim() || starting}>
          {t("import.submit")}
        </Button>
      </div>

      {error && (
        <p role="alert" className="mt-2 text-sm text-danger">
          {error}
        </p>
      )}

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
                className="rounded-xl border border-line bg-bg-soft p-3"
              >
                <div className="flex items-baseline justify-between gap-3">
                  <span className="min-w-0 truncate text-sm font-medium">
                    {job.title || baseName(job.source)}
                  </span>
                  <span
                    className={
                      job.state === "failed" ? "text-[13px] text-danger" : "text-[13px] text-fg-dim"
                    }
                  >
                    {say(describe(job))}
                  </span>
                </div>

                {!isFinished(job) && (
                  <div
                    className="mt-2 h-1 overflow-hidden rounded-full bg-line"
                    role="progressbar"
                    aria-valuenow={pct ?? undefined}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-label={t("import.progress_label", { title: job.title })}
                  >
                    {/* Length unknown: an indeterminate sweep, because a bar frozen at 0% is the
                        one thing a five-minute job must not look like. */}
                    <motion.div
                      className="h-full bg-accent"
                      animate={pct === null ? { x: ["-100%", "100%"] } : { width: `${pct}%` }}
                      transition={
                        pct === null
                          ? { repeat: Infinity, duration: 1.2, ease: "linear" }
                          : METER
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
                    className="mt-2 text-[13px] font-medium text-accent hover:underline"
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
