import { HardDriveDownload } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import { Button, Checkbox, Input } from "../ui";
import { CONTROL, FIELD, HINT, LABEL, SELECT } from "./fields";
import { useT } from "../../i18n/context";
import { LOCAL, type LlmSettings } from "./llm";

/**
 * The model that translates, which is not the model that summarises.
 *
 * A 1B translation model beats a general 8B one at translating, runs on the CPU in under a second a
 * line, and costs nothing — which is what lets somebody with no API key at all still translate
 * every meeting they record. Its own section for the same reason it is its own model: turning
 * translation on is a different decision from choosing who writes your summaries.
 */
export function Translation({ settings }: { settings: LlmSettings }) {
  const navigate = useNavigate();
  const t = useT();
  const { llm, edit, save } = settings;
  if (!llm) return null;

  return (
    <div data-testid="settings-translation">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("settings.mt_hint")}</p>

      <Checkbox
        className="mt-3.5"
        checked={llm.translator != null}
        onChange={(on) =>
          void save({
            ...llm,
            // Turning it on proposes the in-app model, not an endpoint. "Enable this, now go and
            // install a model server" is not a setting anybody finishes, and the whole claim of
            // this feature is that translation costs nothing — which stops being true the moment it
            // depends on a second program.
            translator: on ? { provider: LOCAL, model: "small100" } : null,
          })
        }
      >
        {t("settings.mt_enable")}
      </Checkbox>

      {llm.translator != null && (
        <>
          <label className={FIELD}>
            <span className={LABEL}>{t("settings.mt_where")}</span>
            <select
              className={SELECT}
              value={llm.translator.provider === LOCAL ? LOCAL : "endpoint"}
              aria-label={t("settings.mt_where")}
              onChange={(e) =>
                void save({
                  ...llm,
                  translator:
                    e.target.value === LOCAL
                      ? { provider: LOCAL, model: "small100" }
                      : { provider: "llama-cpp", model: "milmmt-46-1b" },
                })
              }
            >
              <option value={LOCAL}>{t("settings.mt_in_app")}</option>
              <option value="endpoint">{t("settings.mt_endpoint")}</option>
            </select>
          </label>

          {llm.translator.provider !== LOCAL && (
            <label className={FIELD}>
              <span className={LABEL}>{t("settings.endpoint")}</span>
              <Input
                className={CONTROL}
                value={llm.translator.provider}
                aria-label={t("settings.mt_endpoint")}
                placeholder="llama-cpp"
                onChange={(e) =>
                  edit({
                    ...llm,
                    translator: { model: llm.translator?.model ?? null, provider: e.target.value },
                  })
                }
                onBlur={() => void save(llm)}
              />
            </label>
          )}

          <label className={FIELD}>
            <span className={LABEL}>{t("settings.model")}</span>
            <Input
              className={CONTROL}
              value={llm.translator.model ?? ""}
              aria-label={t("settings.mt_model")}
              placeholder="small100"
              onChange={(e) =>
                edit({
                  ...llm,
                  translator: {
                    provider: llm.translator?.provider ?? LOCAL,
                    model: e.target.value,
                  },
                })
              }
              onBlur={() => void save(llm)}
            />
          </label>

          {/* A button, not an instruction. This said "Cài một lần bằng `summo pull small100` —
              611 MB" to somebody using a desktop app: a terminal command, a model id and a size, in
              place of the one thing they wanted, which was to have the model. The catalogue screen
              installs it, shows what it is, how big it is and who wrote it. */}
          {llm.translator.provider === LOCAL ? (
            <Button variant="secondary" size="sm" onClick={() => void navigate({ to: "/models" })}>
              <HardDriveDownload aria-hidden="true" className="me-1.5 size-3.5" />
              {t("settings.mt_pull")}
            </Button>
          ) : (
            <p className={HINT}>{t("settings.mt_run")}</p>
          )}
        </>
      )}
    </div>
  );
}
