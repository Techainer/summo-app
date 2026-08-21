import { useNavigate, useSearch } from "@tanstack/react-router";

import { Transcript } from "../components/Transcript";
import { ImportPanel } from "../components/import/ImportPanel";
import { CaptureControls } from "../components/record/CaptureControls";
import { Recent } from "../components/library/Recent";
import {
  Card,
  CardBody,
  Page,
  PageGlow,
  SectionTitle,
  SegmentedControl,
  Wave,
} from "../components/ui";
import { useT } from "../i18n/context";
import { cn } from "../lib/cn";
import { useEngine } from "../lib/engine-context";
import { formatTime } from "../lib/protocol";

type Source = "record" | "upload";

/**
 * The screen before anything has been said.
 *
 * It used to be one grey sentence in the middle of an empty pane, with the only way to start a
 * recording a small button up in the header — so the screen whose entire job is recording had
 * nothing on it that recorded. This is the button, at the size of the thing it does.
 *
 * The header button stays: a meeting has to be stoppable from wherever the user has navigated to,
 * and this screen is not where they will be. Both drive the same `toggle`, so there is one
 * behaviour and two places to reach it, not two controls that can disagree.
 */
/**
 * The capture panel: the button, the clock, the meter, and what it is listening to.
 *
 * It was a bare circle floating in the middle of an otherwise empty pane, with the source
 * checkboxes drifting on their own a few hundred pixels above it. Three unrelated things, none of
 * them attached to anything — which is what a screen looks like when its parts have no container.
 *
 * One card now holds all of it, in the order somebody uses it: what will be captured, the button,
 * and the meter that proves the microphone is live.
 */
function Idle() {
  const { session, elapsed, level, toggle } = useEngine();
  const t = useT();

  return (
    <Card
      className={cn(
        "relative overflow-hidden transition-shadow duration-300",
        session.recording && "shadow-[var(--glow-rec)]",
      )}
    >
      <div
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-0 bg-[image:var(--gradient-capture)] transition-opacity duration-500",
          session.recording ? "opacity-100" : "opacity-40",
        )}
      />

      <CardBody className="relative flex flex-col gap-6 p-6">
        <div className="flex items-center gap-5">
          <button
            type="button"
            onClick={toggle}
            aria-pressed={session.recording}
            aria-label={session.recording ? t("record.stop") : t("record.start")}
            className={cn(
              "grid size-20 shrink-0 place-items-center rounded-full border-2 transition-all duration-200",
              "focus-visible:ring-accent focus-visible:ring-2 focus-visible:ring-offset-[var(--ring-offset)]",
              "focus-visible:ring-offset-bg focus:outline-none",
              session.recording
                ? "border-rec bg-rec-soft"
                : "border-line-strong bg-bg-elevated hover:border-rec hover:scale-105",
            )}
          >
            {/* A circle that becomes a square, which is what every recorder in the world does. */}
            <span
              aria-hidden="true"
              className={cn(
                "bg-rec transition-all duration-200",
                session.recording ? "size-6 rounded-[6px]" : "size-10 rounded-full",
              )}
            />
          </button>

          <div className="min-w-0">
            <p className="text-title font-semibold">
              {session.recording ? t("record.listening") : t("record.start")}
            </p>
            {session.recording ? (
              <p className="tabular text-display mt-1 font-semibold">{formatTime(elapsed)}</p>
            ) : (
              <p className="text-fg-dim text-meta mt-1 max-w-md leading-relaxed">
                {t("record.idle")}
              </p>
            )}
          </div>
        </div>

        <div
          className={cn(
            "h-20 transition-[color,opacity] duration-300",
            session.recording ? "text-rec" : "text-fg-faint/40",
          )}
          style={{ opacity: session.recording ? 1 : 0.55 + Math.min(0.45, level * 0.9) }}
        >
          {/* Seeded rather than a flat line scaled by `level`: equal bars across a thousand pixels
              read as a ruler rather than a meter. The level moves
              the whole panel's opacity instead, so the microphone still visibly does something. */}
          <Wave seed="record" bars={56} live={session.recording} />
        </div>

        <div className="border-line border-t pt-4">
          <CaptureControls />
        </div>
      </CardBody>
    </Card>
  );
}

/**
 * What the app shows while it is listening, and the two ways to start.
 *
 * The `[Ghi | Nhập file]` control is the entry point from the LetFlow reference, and it is the
 * first thing that tells a new user the app takes files at all — an import buried in a menu is an
 * import nobody finds.
 *
 * Switching to upload while recording is deliberately allowed: the recording keeps running in the
 * daemon, and forbidding it would mean a user who wants to queue a file has to stop their meeting
 * first. The transcript comes back the moment they switch back.
 */
export function RecordScreen() {
  const navigate = useNavigate();
  const { transcript } = useEngine();
  // In the URL, so the panel can be linked to, reloaded into and stepped back out of. `replace`
  // because switching between the two ways in is not a place somebody wants to arrive back at by
  // pressing Back once for each time they changed their mind.
  const source: Source = useSearch({ from: "/record" }).source === "upload" ? "upload" : "record";
  const t = useT();

  return (
    <Page
      fill
      title={t("nav.record")}
      actions={
        <SegmentedControl
          label={t("record.source")}
          size="sm"
          value={source}
          onChange={(next) =>
            void navigate({
              to: "/record",
              search: next === "upload" ? { source: "upload" } : {},
              replace: true,
            })
          }
          options={[
            { value: "record", label: t("record.tab_record") },
            { value: "upload", label: t("record.tab_upload") },
          ]}
        />
      }
    >
      <PageGlow />
      {source === "upload" ? (
        <ImportPanel />
      ) : transcript.segments.length === 0 ? (
        <>
          <Idle />
          {/* What is already here, under the button that makes more of it. Without this the screen
              is one panel and half a window of background — and "open the app, carry on with the
              thing from yesterday" is a more common intent than starting something new. */}
          <section className="flex flex-col gap-2.5">
            <SectionTitle>{t("home.recent")}</SectionTitle>
            <Recent
              limit={3}
              onOpen={(entry) =>
                void navigate({ to: "/pages/$pageId", params: { pageId: entry.id } })
              }
            />
          </section>
        </>
      ) : (
        // Once words are arriving they are the screen: the panel shrinks to its controls and the
        // transcript takes every pixel that is left.
        <div className="flex min-h-0 flex-1 flex-col gap-4">
          <Idle />
          <div className="min-h-0 flex-1">
            <Transcript segments={transcript.segments} />
          </div>
        </div>
      )}
    </Page>
  );
}
