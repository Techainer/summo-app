import { useNavigate } from "@tanstack/react-router";
import {
  BookOpen,
  Bot,
  Cpu,
  FileQuestion,
  FolderOpen,
  Keyboard,
  Languages,
  Mic,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";
import { m } from "motion/react";
import { useState } from "react";

import { Button, Card, CardBody, Page, PageGlow, SectionTitle, Sticker } from "../components/ui";
import { useT } from "../i18n/context";
import { cn } from "../lib/cn";
import { DOCS, ISSUES } from "../lib/menu";
import { listItem, stagger } from "../lib/motion";

/**
 * The manual, inside the app.
 *
 * Everything Summo knows about itself was written down somewhere a user could not reach: the README
 * on GitHub, comments in the source, and a sentence in the corner of whichever screen happened to
 * need it. **Help → Documentation** opened a browser, which is the answer for a developer and not
 * for somebody who has just been asked to allow a microphone.
 *
 * What is here is the small set of things people actually ask, in the order they ask them, each one
 * with the button that acts on it — a page of prose with no way to *do* anything is a page people
 * read once. Where the answer is "look at this screen", the answer is a link to that screen.
 *
 * It is deliberately not everything the README says. A manual nobody finishes is a manual that
 * hides its own useful parts, and the README is one click away at the bottom for the rest.
 */

interface Topic {
  id: string;
  icon: LucideIcon;
  /** Where to go, when the answer is a screen rather than a paragraph. */
  to?: { label: string; go: (navigate: ReturnType<typeof useNavigate>) => void };
}

const TOPICS: Topic[] = [
  {
    id: "privacy",
    icon: ShieldCheck,
  },
  {
    id: "record",
    icon: Mic,
    to: { label: "help.go_record", go: (n) => void n({ to: "/record", search: {} }) },
  },
  {
    id: "models",
    icon: Cpu,
    to: { label: "help.go_models", go: (n) => void n({ to: "/models" }) },
  },
  {
    id: "files",
    icon: FolderOpen,
    to: {
      label: "help.go_storage",
      go: (n) => void n({ to: "/settings", search: { section: "storage" } as const }),
    },
  },
  {
    id: "assistant",
    icon: Bot,
    to: { label: "help.go_assistant", go: (n) => void n({ to: "/chat" }) },
  },
  {
    id: "translate",
    icon: Languages,
    to: {
      label: "help.go_translation",
      go: (n) => void n({ to: "/settings", search: { section: "translation" } as const }),
    },
  },
  {
    id: "shortcuts",
    icon: Keyboard,
  },
  {
    id: "stuck",
    icon: FileQuestion,
  },
];

export function HelpScreen() {
  const t = useT();
  const navigate = useNavigate();
  // Which answer is open. One at a time: eight paragraphs stacked is a wall, and the question is
  // what somebody is scanning for.
  const [open, setOpen] = useState<string | null>("privacy");

  return (
    <Page
      title={t("help.title")}
      subtitle={t("help.lead")}
      aside={<Sticker name="robot" size={72} />}
    >
      <PageGlow />

      <m.div
        initial="hidden"
        animate="shown"
        transition={stagger(TOPICS.length)}
        className="space-y-2"
      >
        {TOPICS.map((topic) => {
          const shown = open === topic.id;
          const Icon = topic.icon;
          return (
            <m.div key={topic.id} variants={listItem}>
              <Card
                className={cn("overflow-hidden transition-colors", shown && "border-accent/40")}
              >
                <button
                  type="button"
                  onClick={() => setOpen(shown ? null : topic.id)}
                  aria-expanded={shown}
                  className="hover:bg-bg-soft/60 flex w-full items-center gap-3 px-4 py-3 text-left transition-colors"
                >
                  <span
                    className={cn(
                      "grid size-8 shrink-0 place-items-center rounded-full transition-colors",
                      shown ? "bg-accent-soft text-accent" : "bg-bg-soft text-fg-faint",
                    )}
                  >
                    <Icon aria-hidden="true" className="size-4" />
                  </span>
                  <span className="flex-1 font-medium">{t(`help.${topic.id}_q`)}</span>
                </button>

                {shown && (
                  <CardBody className="ps-15 pt-0">
                    <p className="text-fg-dim leading-relaxed">{t(`help.${topic.id}_a`)}</p>
                    {topic.to && (
                      <Button
                        size="sm"
                        variant="secondary"
                        className="mt-3"
                        onClick={() => topic.to?.go(navigate)}
                      >
                        {t(topic.to.label)}
                      </Button>
                    )}
                  </CardBody>
                )}
              </Card>
            </m.div>
          );
        })}
      </m.div>

      <section className="mt-8 flex flex-col gap-2.5">
        <SectionTitle>{t("help.more")}</SectionTitle>
        <div className="flex flex-wrap gap-2">
          {/* Out of the app, and said so: these open a browser, which is not what every other button
              on this screen does. */}
          <Button
            size="sm"
            variant="secondary"
            onClick={() => window.open(DOCS, "_blank", "noopener,noreferrer")}
          >
            <BookOpen aria-hidden="true" className="me-1.5 size-3.5" />
            {t("help.readme")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => window.open(ISSUES, "_blank", "noopener,noreferrer")}
          >
            {t("help.issue")}
          </Button>
        </div>
      </section>
    </Page>
  );
}
