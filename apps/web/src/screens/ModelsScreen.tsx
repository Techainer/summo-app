import { CloudOff, HardDriveDownload, Package, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Button, Empty } from "../components/ui";
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
import { OnboardingClient, POLL_MS, isFinished, percent, type Install } from "../lib/onboarding";

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

  useEffect(() => {
    void load();
  }, [load]);

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
    return <p className="text-fg-faint mt-24 text-center">{error ?? t("common.loading")}</p>;
  }

  const groups = byTask(models);

  return (
    <div className="p-5" data-testid="models">
      <h1 className="text-xl font-semibold tracking-tight">{t("models.title")}</h1>
      <p className="text-fg-faint mt-2 max-w-2xl text-[13px] leading-relaxed">{t("models.hint")}</p>
      {/* What they cost, which is why somebody opens this screen a second time. */}
      {installedBytes(models) > 0 && (
        <p className="text-fg-dim tabular mt-1.5 text-[12px]">
          {t("models.on_disk", { size: size(installedBytes(models)) })}
        </p>
      )}

      {!reachable && (
        <p className="border-blocked/30 bg-blocked-soft text-blocked mt-4 flex items-center gap-2 rounded-lg border px-3 py-2 text-[13px]">
          <CloudOff aria-hidden="true" className="size-4 shrink-0" />
          {t("models.offline")}
        </p>
      )}
      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec mt-4 rounded-lg border px-3 py-2 text-[13px]">
          {error}
        </p>
      )}

      {groups.length === 0 ? (
        <Empty icon={Package} title={t("models.none")} hint={t("models.offline")} />
      ) : (
        groups.map((group) => (
          <section key={group.task} className="mt-7">
            <h2 className="text-fg-faint text-[11px] font-semibold tracking-wider uppercase">
              {t(`models.task_${group.task.replace("-", "_")}`)}
            </h2>
            <div className="mt-2.5 grid gap-2.5 lg:grid-cols-2">
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
            </div>
          </section>
        ))
      )}
    </div>
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
  const running = job !== undefined && !isFinished(job);
  const done = percent(job ?? ({} as Install));

  return (
    <article className="border-line bg-bg-soft/40 rounded-[var(--radius-card)] border p-3.5">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="text-[15px] leading-snug font-medium">{model.name}</h3>
          <p className="text-fg-faint tabular mt-0.5 text-[12px]">
            {model.id}
            {model.size_bytes > 0 && ` · ${size(model.size_bytes)}`}
          </p>
        </div>
        {model.installed ? (
          <div className="flex shrink-0 items-center gap-2">
            {/* Installed and *chosen* are different states, and conflating them is what made the
                catalogue decorative: a user could install a Japanese model and record in
                Vietnamese with no indication of why. */}
            {inUse ? (
              <span className="text-accent text-[12px] font-medium">{t("models.in_use")}</span>
            ) : (
              roleFor(model.task) !== null && (
                <Button size="sm" variant="secondary" onClick={onUse}>
                  {t("models.use")}
                </Button>
              )
            )}
            {!inUse && (
              <span className="text-done text-[12px] font-medium">{t("models.installed")}</span>
            )}
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

      {model.description && (
        <p className="text-fg-dim mt-2 text-[13px] leading-relaxed">{model.description}</p>
      )}

      <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
        {tags(model).map((tag) => (
          <span
            key={tag.label}
            className={cn(
              "rounded-full border px-2 py-0.5 text-[11px]",
              tag.kind === "warn" && "border-blocked/40 text-blocked",
              tag.kind === "good" && "border-accent/40 text-accent",
              tag.kind === "plain" && "border-line text-fg-faint",
            )}
          >
            {tag.label}
          </span>
        ))}
      </div>

      {/* The two facts that cost an afternoon if they turn up at the download instead of here. */}
      {!model.fits && (
        <p className="text-blocked mt-2 text-[12px]">
          {t("models.needs_ram", { mb: model.min_ram_mb })}
        </p>
      )}
      {!model.redistributable && !model.installed && (
        <p className="text-fg-faint mt-2 text-[12px]">{t("models.upstream")}</p>
      )}

      {running && (
        <div className="mt-3">
          <div className="bg-bg-soft h-1.5 overflow-hidden rounded-full">
            <div
              className="bg-accent h-full rounded-full transition-[width] duration-300"
              style={{ width: `${done ?? 0}%` }}
            />
          </div>
          <p className="text-fg-faint tabular mt-1 text-[11px]">
            {done === null ? t("models.starting") : `${done}%`}
          </p>
        </div>
      )}
      {job?.state === "failed" && (
        <p className="text-rec mt-2 text-[12px]">{job.error ?? t("models.failed")}</p>
      )}
    </article>
  );
}
