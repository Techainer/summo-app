import { Link } from "@tanstack/react-router";
import {
  AudioLines,
  Bot,
  HardDrive,
  Info,
  Languages,
  Mic,
  Package,
  SlidersHorizontal,
  type LucideIcon,
} from "lucide-react";
import { m } from "motion/react";

import { About } from "./About";
import { General } from "./settings/General";
import { Intelligence } from "./settings/Intelligence";
import { Recording } from "./settings/Recording";
import { Storage } from "./settings/Storage";
import { Translation } from "./settings/Translation";
import { useLlm } from "./settings/llm";
import { cn } from "../lib/cn";
import { useI18n, useT } from "../i18n/context";
import { useIsNarrow } from "../lib/breakpoint";
import { GENTLE, screen as screenVariants } from "../lib/motion";
import type { Handshake } from "../lib/engine";
import type { SectionId } from "../lib/settings";

/**
 * Settings, as a place rather than a scroll.
 *
 * This was one narrow column six screenfuls long: interface language, appearance, microphone
 * permission, spoken language, the summarising model with eight fields of its own, the translation
 * model with five more, and the version at the bottom. Everything was always mounted, everything
 * was always fetched, and finding the one setting somebody came for meant scrolling past every
 * setting they did not.
 *
 * Sections instead, with a rail down the side: six places, each of which fits on a screen and can
 * be linked to. `?section=` is in the URL because "open Settings → Storage" has to be a link — the
 * onboarding checklist, a nudge about disk space and the ⌘K palette all want to send somebody
 * *somewhere* in here rather than to the top of it.
 */

interface Section {
  id: SectionId;
  icon: LucideIcon;
  /** Translation key, resolved at render — a module-level array is built before a provider exists. */
  labelKey: string;
}

/**
 * The order is what a person needs in the order they need it.
 *
 * Language and appearance first because they are what somebody changes on the first day; recording
 * second because it is what they check when a recording produced nothing; the models after that;
 * storage near the end because it is a question you have months in; the version last, where every
 * application in the world keeps it.
 */
const SECTIONS: Section[] = [
  { id: "general", icon: Languages, labelKey: "settings.section_general" },
  { id: "recording", icon: Mic, labelKey: "settings.section_recording" },
  { id: "ai", icon: Bot, labelKey: "settings.section_ai" },
  { id: "translation", icon: SlidersHorizontal, labelKey: "settings.section_translation" },
  { id: "storage", icon: HardDrive, labelKey: "settings.section_storage" },
  { id: "about", icon: Info, labelKey: "settings.section_about" },
];

/**
 * The three screens that belong with the settings and are not settings panels.
 *
 * The sidebar had eleven rows, and three of them — the voice book, the agent roster, the model
 * catalogue — are things somebody sets up once and then does not look at for a month. They sat
 * beside the places where the work happens, which is what made the navigation long enough that
 * nobody read it.
 *
 * Listed here as destinations rather than folded in as panels. Each is a full screen with its own
 * layout, its own empty state and its own toolbar; squeezing them into a 720-pixel settings column
 * would cost all three of those to save one click. This is an index, and the screens stay
 * themselves.
 */
const ELSEWHERE: { to: string; icon: LucideIcon; labelKey: string }[] = [
  { to: "/people", icon: AudioLines, labelKey: "nav.people" },
  { to: "/agents", icon: Bot, labelKey: "nav.agents" },
  { to: "/models", icon: Package, labelKey: "nav.models" },
];

export function Settings({
  handshake,
  section = "general",
  onSection,
}: {
  handshake: Handshake;
  section?: SectionId;
  /** The route owns which section is open, so it survives a reload and the back button. */
  onSection?: (section: SectionId) => void;
}) {
  const t = useT();
  const { locale } = useI18n();
  const narrow = useIsNarrow();
  // One fetch of the model settings for the two sections that show them: split state is how a
  // screen sends a stale translator alongside a fresh provider and undoes half of what was just
  // done. See `settings/llm.ts`.
  const llm = useLlm(handshake);

  const rail = (
    <nav
      aria-label={t("nav.settings")}
      data-testid="settings-nav"
      className={cn(
        "flex gap-1",
        narrow
          ? // A row that scrolls, not a wrapped grid: six pills wrapped to three lines is a third
            // of a phone screen spent on navigation before a single setting is visible.
            "border-line -mx-4 overflow-x-auto border-b px-4 pb-2"
          : "border-line w-[210px] shrink-0 flex-col border-e p-3",
      )}
    >
      {SECTIONS.map(({ id, icon: Icon, labelKey }) => {
        const active = id === section;
        return (
          <button
            key={id}
            type="button"
            data-testid={`settings-tab-${id}`}
            // `page`, because this rail navigates: a screen reader announcing "current page" is the
            // only cue that the highlighted pill is where you already are.
            aria-current={active ? "page" : undefined}
            onClick={() => onSection?.(id)}
            className={cn(
              "text-meta flex items-center gap-2.5 rounded-[var(--radius-card)] px-3 py-2 text-start transition-colors",
              narrow && "shrink-0 whitespace-nowrap",
              active
                ? "bg-bg-raised text-fg font-medium shadow-[var(--shadow-sm)]"
                : "text-fg-dim hover:bg-bg-soft hover:text-fg",
            )}
          >
            <Icon aria-hidden="true" className="size-4 shrink-0 stroke-[1.75]" />
            {t(labelKey)}
          </button>
        );
      })}
      {/* A rule, because what follows is a different kind of thing: above are panels that change
          when you press them, below are screens you leave for. Without it the rail reads as nine
          settings, three of which mysteriously navigate. */}
      <div className={cn("border-line", narrow ? "border-s ms-1 me-1" : "mt-2 border-t pt-2")} />

      {/* The three screens this rail indexes rather than contains. Drawn as links, so a right
          click opens one in its own window and the browser's own affordances work. */}
      {ELSEWHERE.map(({ to, icon: Icon, labelKey }) => (
        <Link
          key={to}
          to={to}
          search={{}}
          className={cn(
            "text-meta text-fg-dim hover:bg-bg-soft hover:text-fg flex items-center gap-2.5 rounded-[var(--radius-card)] px-3 py-2 text-start transition-colors",
            narrow && "shrink-0 whitespace-nowrap",
          )}
        >
          <Icon aria-hidden="true" className="size-4 shrink-0 stroke-[1.75]" />
          {t(labelKey)}
        </Link>
      ))}

      {/* Where the app is, at the bottom of the rail. Every setting on these six panels is stored
          under that folder, and "where does this keep my meetings" is the question a local-first
          app is asked first — on a screen where the answer is one line, not a support article. */}
      {!narrow && (
        <p className="text-fg-faint text-micro border-line mt-auto border-t px-3 pt-3 leading-normal">
          {t("settings.vault_at")}
          <br />
          <span className="text-fg-dim">~/.summo</span>
        </p>
      )}
    </nav>
  );

  return (
    <div
      className={cn("flex h-full min-h-0", narrow ? "flex-col px-4 py-4" : "flex-row")}
      data-testid="settings"
    >
      {rail}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* Keyed on the section so the panel animates on every change rather than only the first:
            the rail is a small target and the movement is what confirms the click landed. */}
        <m.div
          key={section}
          variants={screenVariants}
          initial="hidden"
          animate="shown"
          transition={GENTLE}
          className={cn("mx-auto w-full max-w-[720px]", narrow ? "pt-4" : "px-7 py-6")}
          lang={locale}
        >
          {/* One heading, drawn here rather than by each section: six panels each rendering their
              own title is six chances for one of them to be a different size. */}
          <h2 className="text-title mb-3 font-semibold tracking-tight">
            {t(SECTIONS.find((one) => one.id === section)?.labelKey ?? "nav.settings")}
          </h2>
          {section === "general" && <General />}
          {section === "recording" && <Recording />}
          {section === "ai" && <Intelligence settings={llm} />}
          {section === "translation" && <Translation settings={llm} />}
          {section === "storage" && <Storage handshake={handshake} />}
          {section === "about" && <About />}
        </m.div>
      </div>
    </div>
  );
}
