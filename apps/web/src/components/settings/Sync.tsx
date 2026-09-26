import { FolderSync, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useT } from "../../i18n/context";
import type { Handshake } from "../../lib/engine";
import { useErrorText } from "../../lib/errors";
import { pickFolder } from "../../lib/folder";
import { SyncClient, actionKey, isQuiet, parts, type Report, type SyncState } from "../../lib/sync";
import { Alert, Button, Input, SectionTitle } from "../ui";
import { CONTROL, FIELD, HINT, LABEL } from "./fields";

/**
 * Keeping this vault in step with a folder.
 *
 * `summo-sync` has had ninety-four tests and one caller since it landed: `summo sync`, a
 * subcommand. The product page has advertised "encrypted sync between your machines through any
 * shared folder" the whole time, and from inside the app there was no folder to choose, no button
 * to press and no way to find out it existed.
 *
 * ## Two steps, because the first sync moves everything
 *
 * "See what would happen" asks the daemon to plan without writing — which is a dry run on its side
 * too, the same one `summo sync --dry-run` does. It matters most on the first run: an existing
 * vault uploads every file, and somebody should be able to look at that list before it happens.
 *
 * ## The passphrase is typed every time
 *
 * There is no "remember me" and the field is cleared after each run. It is the only thing between
 * whoever holds the folder and every meeting in the vault; storing it beside the folder path would
 * be handing over both halves. The command line refuses it as an argument for the same reason — an
 * argument is in `ps` and in the shell history.
 */
export function Sync({ handshake }: { handshake: Handshake }) {
  const t = useT();
  const errorText = useErrorText();
  const client = useMemo(() => new SyncClient(handshake), [handshake]);

  const [state, setState] = useState<SyncState | null>(null);
  const [folder, setFolder] = useState("");
  const [machine, setMachine] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState<"plan" | "run" | null>(null);
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    client
      .state()
      .then((next) => {
        setState(next);
        setFolder(next.folder);
        setMachine(next.machine);
      })
      .catch((e: unknown) => setError(errorText(e)));
  }, [client, errorText]);

  useEffect(load, [load]);

  const save = async (nextFolder: string, nextMachine: string) => {
    setError(null);
    try {
      await client.configure(nextFolder, nextMachine);
      load();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const browse = async () => {
    const chosen = await pickFolder(t("sync.pick_folder"));
    // Outside the desktop shell there is no dialog. The field stays the way in, so say that once
    // rather than leaving a button that appears to do nothing.
    if (chosen === null) {
      setError(t("sync.no_dialog"));
      return;
    }
    setFolder(chosen);
    await save(chosen, machine);
  };

  const go = (dryRun: boolean) => {
    setBusy(dryRun ? "plan" : "run");
    setError(null);
    setReport(null);
    void (async () => {
      try {
        setReport(await client.run(passphrase, dryRun));
        if (!dryRun) {
          // Cleared as soon as it has been used. Left in the field it would sit in a form control
          // for as long as this screen is open, where a screenshot or a shoulder picks it up.
          setPassphrase("");
          load();
        }
      } catch (e) {
        setError(errorText(e));
      } finally {
        setBusy(null);
      }
    })();
  };

  const ready = folder.trim() !== "" && passphrase.trim() !== "" && busy === null;

  return (
    <div data-testid="settings-sync">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("sync.hint")}</p>

      {/* Three things on one row, which is one more than `FIELD` was drawn for — so the control
          and the button share the space the control alone usually gets.

          `FIELD` stacks below `lg` now, which is what makes this safe: on a tablet the label sits
          above and the input and button have the full width between them. It did not, and at 768
          this row gave the path input about seventy pixels — `/mnt/` and nothing else. */}
      <div className={FIELD}>
        <span className={LABEL} id="sync-folder-label">
          {t("sync.folder")}
        </span>
        <div className="flex w-full min-w-0 items-center gap-2 lg:flex-1">
          <Input
            className="min-w-0 flex-1"
            data-testid="sync-folder"
            value={folder}
            placeholder={t("sync.folder_placeholder")}
            aria-labelledby="sync-folder-label"
            onChange={(e) => setFolder(e.target.value)}
            onBlur={() => void save(folder, machine)}
          />
          <Button size="sm" variant="secondary" className="shrink-0" onClick={() => void browse()}>
            {t("sync.browse")}
          </Button>
        </div>
      </div>
      <p className={HINT}>{t("sync.folder_hint")}</p>

      {/* Chosen and unreachable. Not the same as "not set up", and the state this is in most
          often — an unmounted drive, a stick somebody pulled out. */}
      {state?.problem && (
        <Alert tone="rec" className="mt-3" data-testid="sync-problem">
          {state.problem}
        </Alert>
      )}

      <label className={FIELD}>
        <span className={LABEL}>{t("sync.machine")}</span>
        <Input
          className={CONTROL}
          data-testid="sync-machine"
          value={machine}
          aria-label={t("sync.machine")}
          onChange={(e) => setMachine(e.target.value)}
          onBlur={() => void save(folder, machine)}
        />
      </label>
      <p className={HINT}>{t("sync.machine_hint")}</p>

      <label className={FIELD}>
        <span className={LABEL}>{t("sync.passphrase")}</span>
        <Input
          type="password"
          className={CONTROL}
          data-testid="sync-passphrase"
          value={passphrase}
          aria-label={t("sync.passphrase")}
          autoComplete="off"
          onChange={(e) => setPassphrase(e.target.value)}
        />
      </label>
      <p className={HINT}>{t("sync.passphrase_hint")}</p>

      {/* The first sync of an existing vault uploads every file. Said before the button, not
          after — it is the one thing somebody would have wanted to know in advance. */}
      {state && !state.synced_before && folder.trim() !== "" && (
        <p className="text-fg-dim text-meta mt-4">{t("sync.first_run")}</p>
      )}

      <div className="mt-5 flex flex-wrap items-center gap-3">
        <Button
          variant="ghost"
          onClick={() => go(true)}
          disabled={!ready}
          busy={busy === "plan"}
          data-testid="sync-plan"
        >
          <FolderSync aria-hidden="true" className="size-4" />
          {t("sync.plan")}
        </Button>
        <Button
          onClick={() => go(false)}
          disabled={!ready}
          busy={busy === "run"}
          data-testid="sync-run"
        >
          <RefreshCw aria-hidden="true" className="size-4" />
          {t("sync.run")}
        </Button>
      </div>

      {report && <Outcome report={report} />}

      {error && (
        <Alert tone="rec" className="mt-3" data-testid="sync-error">
          {error}
        </Alert>
      )}
    </div>
  );
}

/** What a run or a plan turned out to be. */
function Outcome({ report }: { report: Report }) {
  const t = useT();
  const counted = parts(report.summary);

  return (
    <div className="mt-4" data-testid="sync-report">
      <p className="text-meta">
        {isQuiet(report.summary)
          ? t(report.applied ? "sync.nothing_to_do" : "sync.plan_nothing")
          : [
              t(report.applied ? "sync.did" : "sync.would"),
              counted.map((part) => t(part.key, { count: part.count })).join(", "),
            ].join(" ")}
      </p>

      {/* Only a plan lists the files. After a real run the same list would be a wall of text
          describing work that has already happened — the counts above are what a reader checks. */}
      {report.steps.length > 0 && (
        <>
          <SectionTitle className="mt-4">{t("sync.what_would_happen")}</SectionTitle>
          <ul className="mt-2 flex flex-col">
            {report.steps.slice(0, 20).map((step) => (
              <li
                key={`${step.action}:${step.path}`}
                className="border-line text-meta flex items-baseline gap-3 border-b py-1.5 last:border-b-0"
              >
                <span className="text-fg-faint text-micro w-28 shrink-0">{t(actionKey(step))}</span>
                <span className="min-w-0 flex-1 truncate">{step.path}</span>
              </li>
            ))}
          </ul>
          {/* Said, not silently cut. A list that stops at twenty and does not mention it reads as
              "that was all of it", and on a first sync it is twenty out of ten thousand. */}
          {report.steps.length > 20 && (
            <p className="text-fg-faint text-micro mt-2">
              {t("sync.and_more", { count: report.steps.length - 20 })}
            </p>
          )}
        </>
      )}

      {/* Both sides changed the same file. Not an error — the other version is beside it, whole,
          and which one wins is the user's call. */}
      {report.conflicts.length > 0 && (
        <div className="mt-4">
          <SectionTitle>{t("sync.conflicts")}</SectionTitle>
          <ul className="mt-2 flex flex-col">
            {report.conflicts.map((conflict) => (
              <li key={conflict.path} className="text-meta py-1">
                {t("sync.conflict_line", { path: conflict.path, copy: conflict.copy })}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* A path from the folder that would have been written outside the vault. Reported and never
          acted on — and worth naming, because a folder producing these is a folder somebody else
          has been writing to. */}
      {report.refused.length > 0 && (
        <Alert tone="rec" className="mt-4">
          {t("sync.refused", { count: report.refused.length, paths: report.refused.join(", ") })}
        </Alert>
      )}
    </div>
  );
}
