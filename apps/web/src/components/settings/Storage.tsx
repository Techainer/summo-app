import { HardDrive, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Button, Card, CardBody, Checkbox, Input, SectionTitle } from "../ui";
import { FIELD, HINT, LABEL } from "./fields";
import { useI18n, useT } from "../../i18n/context";
import { StorageClient, bytes, type Pruned, type Usage } from "../../lib/storage";
import type { Handshake } from "../../lib/engine";

/**
 * What Summo is using on disk, and getting it back.
 *
 * The daemon has measured this since the vault was written: `/storage` reports it, `/storage/prune`
 * enforces the retention setting, and `settings.storage` decides both. None of it was reachable
 * from the app — the setting could only be changed by editing `~/.summo/settings.toml`, and the
 * question it answers ("how long do you keep my recordings?") is the one a local-first recorder is
 * most obliged to answer on screen.
 *
 * Deleting is the only irreversible thing in this app, so it happens in two steps: what *would* go,
 * then a confirmation. That is also how the daemon behaves — a prune with no parameter is a dry run
 * — and the screen does not paper over it.
 */
export function Storage({ handshake }: { handshake: Handshake }) {
  const t = useT();
  const { locale } = useI18n();
  const client = useMemo(() => new StorageClient(handshake), [handshake]);
  const [usage, setUsage] = useState<Usage | null>(null);
  const [keepDays, setKeepDays] = useState<number | null>(null);
  const [keepAudio, setKeepAudio] = useState(true);
  const [planned, setPlanned] = useState<Pruned | null>(null);
  const [done, setDone] = useState<Pruned | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const say = (e: unknown) => setError(e instanceof Error ? e.message : String(e));

  const load = useCallback(() => {
    client.usage().then(setUsage).catch(say);
  }, [client]);

  useEffect(() => {
    load();
    // The policy comes from the same place every other setting does, so this screen and the file
    // cannot disagree about what is set.
    fetch(`http://127.0.0.1:${handshake.port}/settings?token=${handshake.token}`)
      .then((r) => r.json())
      .then(
        (body: {
          settings: { storage: { audio_retention_days: number; keep_audio: boolean } };
        }) => {
          setKeepDays(body.settings.storage.audio_retention_days);
          setKeepAudio(body.settings.storage.keep_audio);
        },
      )
      .catch(say);
  }, [handshake, load]);

  const policy = useCallback(
    async (next: { keep_days?: number; keep_audio?: boolean }) => {
      try {
        const saved = await client.policy(next);
        setKeepDays(saved.keep_days);
        setKeepAudio(saved.keep_audio);
        setPlanned(null);
      } catch (e) {
        say(e);
      }
    },
    [client],
  );

  const plan = useCallback(async () => {
    setBusy(true);
    setDone(null);
    try {
      setPlanned(await client.prune(true));
    } catch (e) {
      say(e);
    } finally {
      setBusy(false);
    }
  }, [client]);

  const confirm = useCallback(async () => {
    setBusy(true);
    try {
      setDone(await client.prune(false));
      setPlanned(null);
      load();
    } catch (e) {
      say(e);
    } finally {
      setBusy(false);
    }
  }, [client, load]);

  return (
    <div data-testid="settings-storage">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("storage.hint")}</p>

      {/* The four numbers, before any control that changes them. Somebody opening this screen is
          answering "what is taking the space", and the answer is nearly always the audio. */}
      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        {(
          [
            ["storage.audio", usage?.audio_bytes],
            ["storage.vault", usage?.vault_bytes],
            ["storage.models", usage?.model_bytes],
            ["storage.total", usage?.total_bytes],
          ] as const
        ).map(([key, value]) => (
          <Card key={key}>
            <CardBody className="p-3.5">
              <p className="text-fg-faint text-micro">{t(key)}</p>
              <p className="text-title nums mt-0.5 font-semibold" data-testid={`usage-${key}`}>
                {value === undefined ? "—" : bytes(value, locale)}
              </p>
            </CardBody>
          </Card>
        ))}
      </div>

      <div className="mt-6">
        <Checkbox
          checked={keepAudio}
          onChange={(on) => void policy({ keep_audio: on })}
          data-testid="keep-audio"
        >
          {t("storage.keep_audio")}
        </Checkbox>
        <p className="text-fg-faint text-micro mt-1.5 ml-7 leading-normal">
          {t("storage.keep_audio_hint")}
        </p>
      </div>

      <label className={FIELD}>
        <span className={LABEL}>{t("storage.keep_days")}</span>
        <Input
          type="number"
          min={0}
          className="w-28"
          data-testid="keep-days"
          value={keepDays ?? ""}
          aria-label={t("storage.keep_days")}
          onChange={(e) => setKeepDays(Number(e.target.value))}
          onBlur={() => keepDays !== null && void policy({ keep_days: Math.max(0, keepDays) })}
        />
      </label>
      <p className={HINT}>{keepDays === 0 ? t("storage.forever") : t("storage.keep_days_hint")}</p>

      {/* Two steps, because this is the only thing in the app that cannot be undone. The first
          button asks the daemon what *would* go — which is a dry run on its side too. */}
      <div className="mt-5 flex flex-wrap items-center gap-3">
        <Button
          variant="ghost"
          onClick={() => void plan()}
          disabled={busy}
          data-testid="plan-prune"
        >
          <HardDrive aria-hidden="true" className="size-4" />
          {t("storage.check")}
        </Button>
        {planned && planned.freed_bytes > 0 && (
          <Button
            variant="danger"
            onClick={() => void confirm()}
            disabled={busy}
            data-testid="do-prune"
          >
            <Trash2 aria-hidden="true" className="size-4" />
            {t("storage.delete", { size: bytes(planned.freed_bytes, locale) })}
          </Button>
        )}
      </div>

      {planned && (
        <p className="text-meta mt-3" data-testid="prune-plan">
          {planned.freed_bytes > 0
            ? t("storage.would_free", {
                size: bytes(planned.freed_bytes, locale),
                count: planned.removed.length,
              })
            : t("storage.nothing_to_free")}
        </p>
      )}
      {done && (
        <p className="border-accent/30 bg-accent-soft text-meta mt-3 rounded-lg border px-3 py-2">
          {t("storage.freed", { size: bytes(done.freed_bytes, locale) })}
        </p>
      )}
      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta mt-3 rounded-lg border px-3 py-2">
          {error}
        </p>
      )}

      {/* Which meetings the space is in. Largest first, as the daemon sorted them, because the
          question under "why is this 4 GB" is nearly always one long recording. */}
      {usage && usage.recordings.length > 0 && (
        <>
          <SectionTitle className="mt-8">{t("storage.biggest")}</SectionTitle>
          <ul className="mt-2 flex flex-col">
            {usage.recordings.slice(0, 8).map((recording) => (
              <li
                key={recording.id}
                data-testid="storage-recording"
                className="border-line text-meta flex items-baseline gap-3 border-b py-2 last:border-b-0"
              >
                <span className="flex-1 truncate">{recording.title || t("storage.orphan")}</span>
                <span className="text-fg-faint text-micro nums">{recording.day}</span>
                <span className="nums tabular w-20 text-end">{bytes(recording.bytes, locale)}</span>
              </li>
            ))}
          </ul>
        </>
      )}

      {/* Audio with no meeting left to explain it: a deleted transcript, or a crash between the two
          writes. Worth naming rather than folding into the total — it is space nothing will ever
          use again. */}
      {usage && usage.orphaned.length > 0 && (
        <p className="text-fg-faint text-micro mt-4" data-testid="orphaned">
          {t("storage.orphaned", {
            count: usage.orphaned.length,
            size: bytes(
              usage.orphaned.reduce((sum, one) => sum + one.bytes, 0),
              locale,
            ),
          })}
        </p>
      )}
    </div>
  );
}
