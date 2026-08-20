import { CloudOff, HardDriveDownload, Package, Trash2 } from "lucide-react";
import { m } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Markdown } from "../components/page/Markdown";
import { Button, Empty, Page, PageGlow, SectionTitle } from "../components/ui";
import { useT } from "../i18n/context";
import { cn } from "../lib/cn";
import {
  CatalogueClient,
  byTask,
  installedBytes,
  roleFor,
  size,
  tags,
  type CatalogueModel,
} from "../lib/catalogue";
import { useEngine } from "../lib/engine-context";
import { useErrorText } from "../lib/errors";
import { listItem, stagger } from "../lib/motion";
import { OnboardingClient, POLL_MS, isFinished, percent, type Install } from "../lib/onboarding";
import { url } from "../lib/library";
import { useRefresh } from "../lib/use-load";

/**
 * Every model, what it is for, and a button.
 *
 * The registry has always been able to answer this and nothing ever asked it: the only way to
 * install a model that was not the recommended one was `summo pull` on a command line. A local-first
 * app whose whole promise is which models run on your machine has to let you see them.
 *
 * Three decisions:
 *
 * **Grouped by what the model does**, not by name or size. "Which speech model" and "which
 * translator" are different questions asked at different times, and a flat list makes both harder.
 *
 * **The facts that change a decision, on the card.** Size, licence, languages — and the two that
 * cost a user an afternoon if they find out at the download instead: `gated` means an account is
 * needed upstream, `upstream` means Summo does not host it.
 *
 * **An unreachable registry is a state, not an error.** The screen shows what is installed and says
 * the catalogue is offline. Going blank without a network would be worse, and this is an app that
 * is expected to work on a plane.
 */
export function ModelsScreen() {
  const t = useT();
  const say = useErrorText();
  const { handshake } = useEngine();
  const catalogue = useMemo(() => new CatalogueClient(handshake), [handshake]);
  const installer = useMemo(() => new OnboardingClient(handshake), [handshake]);

  const [models, setModels] = useState<CatalogueModel[] | null>(null);
  const [reachable, setReachable] = useState(true);
  const [installs, setInstalls] = useState<Install[]>([]);
  /** Which model fills each role, so the card can say "in use" rather than offering a button. */
  const [chosen, setChosen] = useState<Record<string, string | null>>({});
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [next, running] = await Promise.all([catalogue.load(), installer.installs()]);
      setModels(next.models);
      setReachable(next.reachable);
      setInstalls(running);
      setChosen(next.chosen ?? {});
      setError(null);
    } catch (e) {
      // The daemon answers with `reachable: false` when the *registry* is unreachable, so getting
      // here means the daemon itself did not answer. Either way the honest screen is the same one:
      // what is installed, and a line saying the catalogue is not available. Showing a bare fetch
      // error instead would be technically accurate and useless.
      setModels((current) => current ?? []);
      setReachable(false);
      setError(say(e));
    }
  }, [catalogue, installer, say]);

  useRefresh(load);

  // While something is downloading, and only then. Polling an idle screen every second is a
  // request per second for a list that has not changed.
  const downloading = installs.some((job) => !isFinished(job));
  useEffect(() => {
    if (!downloading) return undefined;
    const timer = window.setInterval(() => void load(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [downloading, load]);

  const remove = useCallback(
    async (id: string) => {
      try {
        await catalogue.remove(id);
        await load();
        setError(null);
      } catch (e) {
        // The daemon refuses to remove a model the settings point at, and says which role it
        // fills. That message is the whole answer, so it is shown rather than summarised.
        setError(say(e));
      }
    },
    [catalogue, load, say],
  );

  const choose = useCallback(
    async (model: CatalogueModel) => {
      const role = roleFor(model.task);
      if (!role) return;
      try {
        await catalogue.use(role, model.id);
        setChosen((current) => ({ ...current, [role]: model.id }));
        setError(null);
      } catch (e) {
        setError(say(e));
      }
    },
    [catalogue, say],
  );

  const pull = useCallback(
    async (id: string) => {
      try {
        const job = await installer.install(id);
        setInstalls((current) => [...current.filter((i) => i.model !== id), job]);
      } catch (e) {
        setError(say(e));
      }
    },
    [installer, say],
  );

  if (!models) {
    return (
      <p className="text-fg-faint grid h-full place-items-center text-center">
        {error ?? t("common.loading")}
      </p>
    );
  }

  const groups = byTask(models);

  return (
    <Page title={t("models.title")} subtitle={t("models.hint")} data-testid="models">
      <PageGlow />
      {/* What they cost, which is why somebody opens this screen a second time. */}
      {installedBytes(models) > 0 && (
        <p className="text-fg-dim nums text-micro -mt-4">
          {t("models.on_disk", { size: size(installedBytes(models)) })}
        </p>
      )}

      {!reachable && (
        <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta mt-4 flex items-center gap-2 rounded-lg border px-3 py-2">
          <CloudOff aria-hidden="true" className="size-4 shrink-0" />
          {t("models.offline")}
        </p>
      )}
      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta mt-4 rounded-lg border px-3 py-2">
          {error}
        </p>
      )}

      {groups.length === 0 ? (
        <Empty icon={Package} title={t("models.none")} hint={t("models.offline")} />
      ) : (
        groups.map((group) => (
          <section key={group.task}>
            <SectionTitle>{t(`models.task_${group.task.replace("-", "_")}`)}</SectionTitle>
            {/* Cards arrive rather than appear. A grid of eight that pops in at once reads as a
                page repainting; a short stagger reads as a list being laid out, and it gives the
                eye an order to follow. `stagger` shortens the step as the count grows, so a long
                section does not take a second to finish. */}
            <m.div
              initial="hidden"
              animate="shown"
              transition={stagger(group.models.length)}
              className="mt-2.5 grid gap-2.5 lg:grid-cols-2"
            >
              {group.models.map((model) => (
                <Card
                  key={model.id}
                  model={model}
                  job={installs.find((job) => job.model === model.id)}
                  onPull={() => void pull(model.id)}
                  onRemove={() => void remove(model.id)}
                  onUse={() => void choose(model)}
                  inUse={chosen[roleFor(model.task) ?? ""] === model.id}
                />
              ))}
            </m.div>
          </section>
        ))
      )}
    </Page>
  );
}

function Card({
  model,
  job,
  onPull,
  onRemove,
  onUse,
  inUse,
}: {
  model: CatalogueModel;
  job: Install | undefined;
  onPull: () => void;
  onRemove: () => void;
  onUse: () => void;
  inUse: boolean;
}) {
  const t = useT();
  // Two clicks, not a dialog. Re-downloading a gigabyte is a real cost, and a modal for it would
  // be one more thing to dismiss on the screen where somebody is tidying up several models.
  const [confirming, setConfirming] = useState(false);
  const [open, setOpen] = useState(false);
  const [page, setPage] = useState<string | null>(null);
  const { handshake } = useEngine();

  // Fetched when it is opened, not with the list: eight model pages, each carrying an upstream
  // README, is a lot of text to download for a screen most people scroll past.
  useEffect(() => {
    if (!open || page !== null) return;
    fetch(url(handshake, `/models/${encodeURIComponent(model.id)}/page`))
      .then((r) => r.json())
      .then((body: { markdown?: string }) => setPage(body.markdown ?? ""))
      .catch(() => setPage(""));
  }, [open, page, handshake, model.id]);

  const running = job !== undefined && !isFinished(job);
  const done = percent(job ?? ({} as Install));

  return (
    <m.article
      variants={listItem}
      className={cn(
        // A real card on the page surface, not a translucent tint of it. Elevation is what tells
        // the eye these are eight separate things to choose between.
        "border-line bg-bg-raised rounded-[var(--radius-card)] border p-4 shadow-[var(--shadow-sm)]",
        // Lifts under the pointer. The whole card is a decision — read it, install it, remove it —
        // so the whole card should acknowledge the cursor rather than only the button on it.
        "transition-[transform,box-shadow,border-color,background-color] duration-150",
        "hover:bg-bg-elevated hover:border-line-strong hover:-translate-y-0.5 hover:shadow-[var(--shadow-card)]",
        inUse && "border-accent/40",
      )}
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-body leading-snug font-medium">{model.name}</h3>
            {/* State on the title line, where the eye already is: whether this machine has it, and
                whether it is the one being used. Those were two buttons and a word in three
                different places on the card. */}
            {inUse ? (
              <span className="border-accent/40 text-accent text-micro rounded-full border px-2 py-0.5">
                {t("models.in_use")}
              </span>
            ) : model.installed ? (
              <span className="border-done/40 text-done text-micro rounded-full border px-2 py-0.5">
                {t("models.installed")}
              </span>
            ) : null}
          </div>
          <p className="text-fg-faint tabular text-micro mt-0.5">
            {model.id}
            {model.size_bytes > 0 && ` · ${size(model.size_bytes)}`}
          </p>
        </div>
        {model.installed ? (
          <div className="flex shrink-0 items-center gap-2">
            {/* Installed and *chosen* are different states, and conflating them is what made the
                catalogue decorative: a user could install a Japanese model and record in
                Vietnamese with no indication of why. */}
            {inUse
              ? null
              : roleFor(model.task) !== null && (
                  <Button size="sm" variant="secondary" onClick={onUse}>
                    {t("models.use")}
                  </Button>
                )}
            {/* No second "Đã cài" here: the state chip lives on the title line now, where the eye
                lands first. Two of them meant the same word twice on one card. */}
            <Button
              size="sm"
              variant={confirming ? "danger" : "ghost"}
              onClick={() => {
                if (confirming) onRemove();
                setConfirming(!confirming);
              }}
              onBlur={() => setConfirming(false)}
            >
              <Trash2 aria-hidden="true" className="me-1 size-3.5" />
              {confirming ? t("models.remove_confirm") : t("models.remove")}
            </Button>
          </div>
        ) : (
          <Button
            size="sm"
            variant="primary"
            busy={running}
            // A model the machine cannot hold is not offered. The download would finish and the
            // load would fail, which is the most expensive possible way to find out.
            disabled={!model.fits}
            onClick={onPull}
          >
            <HardDriveDownload aria-hidden="true" className="me-1 size-3.5" />
            {t("models.install")}
          </Button>
        )}
      </div>

      {/* Clamped, and the registry's own text is not shortened to match.
          A manifest description is written for the model's page — measured numbers, licence
          reasoning, why one model beats another — which is right there and far too long here. One
          card ran to twelve lines beside a neighbour that ran to three, and a two-column grid of
          those is unreadable before a word of it is read. Three lines is enough to decide whether
          to keep reading. */}
      {model.description && (
        <p className="text-fg-dim text-meta mt-2 line-clamp-3 leading-relaxed">
          {model.description}
        </p>
      )}

      <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
        {tags(model).map((tag) => (
          <span
            key={tag.label}
            className={cn(
              "text-micro rounded-full border px-2 py-0.5",
              tag.kind === "warn" && "border-blocked/40 text-blocked",
              tag.kind === "good" && "border-accent/40 text-accent",
              tag.kind === "plain" && "border-line text-fg-faint",
            )}
          >
            {tag.label}
          </span>
        ))}
      </div>

      {/* The rest of what a model card owes a reader: measured accuracy, measured speed, memory,
          checksums, and the publisher's own README. All of it has existed in `summo_models::page`
          since the registry did — rendered for the CLI and never shown here, so somebody choosing
          between two models had a name, a size and three chips to do it with. */}
      <button
        type="button"
        onClick={() => setOpen((was) => !was)}
        aria-expanded={open}
        className="text-fg-faint hover:text-fg text-micro mt-2 underline"
      >
        {open ? t("models.hide_details") : t("models.details")}
      </button>
      {open && (
        <div className="border-line mt-2 border-t pt-2">
          {page === null ? (
            <p className="text-fg-faint text-micro">{t("common.loading")}</p>
          ) : (
            // From the first section down. The page opens with the model's name, its id and its
            // description — all three of which are already on the card this is expanding under,
            // and reading them twice is how a detail view starts to feel like a different screen
            // that happens to be inside this one.
            <Markdown
              markdown={page.slice(Math.max(0, page.indexOf("\n## ")))}
              className="text-meta"
            />
          )}
        </div>
      )}

      {/* The two facts that cost an afternoon if they turn up at the download instead of here. */}
      {!model.fits && (
        <p className="text-blocked text-micro mt-2">
          {t("models.needs_ram", { mb: model.min_ram_mb })}
        </p>
      )}
      {/* Whose model it is. Two rewrites to get here: first a licensing position written for a
          lawyer ("Summo không phân phối mô hình này. File tải thẳng từ nơi phát hành, theo giấy
          phép của họ."), then a sentence about where the bytes travel from, which is not something
          anyone choosing a model needs to think about. A credit is the whole of it. */}
      {model.attribution && !model.installed && (
        <p className="text-fg-faint text-micro mt-2">
          {t("models.upstream", { who: model.attribution || t("models.upstream_who") })}
        </p>
      )}

      {running && (
        <div className="mt-3">
          <div className="bg-bg-soft h-1.5 overflow-hidden rounded-full">
            <div
              className="bg-accent h-full rounded-full transition-[width] duration-300"
              style={{ width: `${done ?? 0}%` }}
            />
          </div>
          <p className="text-fg-faint nums text-micro mt-1">
            {done === null ? t("models.starting") : `${done}%`}
          </p>
        </div>
      )}
      {job?.state === "failed" && (
        <p className="text-rec text-micro mt-2">{job.error ?? t("models.failed")}</p>
      )}
    </m.article>
  );
}
