import { describe, expect, it } from "vitest";

import { byTask, canRun, current, roleFor, type CatalogueModel, type Task } from "./catalogue";

/** A card with only the fields under test filled in. */
function model(over: Partial<CatalogueModel> = {}): CatalogueModel {
  return {
    id: "x",
    name: "X",
    task: "asr",
    mode: "live",
    langs: ["en"],
    license: "MIT",
    redistributable: true,
    gated: false,
    size_bytes: 1,
    installed: false,
    fits: true,
    min_ram_mb: 0,
    ...over,
  };
}

describe("whether this build can run a model", () => {
  /**
   * The case this field was added for.
   *
   * The release ships the ONNX translation runtime and not llama.cpp, so both GGUF translators in
   * the registry — 0.8 GB and 2.4 GB — were offered by every build that could never load them. The
   * download worked, the digest matched, and the failure arrived at the first translation.
   */
  it("refuses a model whose runtime the daemon says it lacks", () => {
    const gguf = model({ runnable: false, why_not: "no runtime for `llama.cpp/gguf`" });
    expect(canRun(gguf)).toBe(false);
  });

  it("allows one the daemon says it can run", () => {
    expect(canRun(model({ runnable: true }))).toBe(true);
  });

  /**
   * An older daemon does not send the field at all.
   *
   * Missing has to read as yes. Being wrong in that direction costs a failed install with the
   * daemon's own message — exactly what happened before this field existed — while being wrong the
   * other way hides every working model behind a dead button.
   */
  it("treats a daemon that does not send the field as able to run everything", () => {
    expect(canRun(model())).toBe(true);
  });
});

describe("which role a task fills", () => {
  /**
   * A voice shipped installable and unchoosable: `summo dub` took a hand-typed id, so the model
   * that most needed a Use button did not get one and publishing it moved the problem rather than
   * solving it.
   */
  it("gives a voice a role, so an installed one can be chosen", () => {
    expect(roleFor("tts")).toBe("tts");
  });

  it("maps the roles a user actually decides between", () => {
    expect(roleFor("asr")).toBe("live");
    expect(roleFor("translate")).toBe("translator");
    expect(roleFor("denoise")).toBe("denoise");
  });

  /**
   * One detector and one embedder exist and the machine picks them; there is nothing to choose. A
   * `null` here is what keeps a "Use" button off a card where pressing it would mean nothing.
   */
  it("offers no role where choosing is not a thing a user does", () => {
    for (const task of ["vad", "speaker-embed", "diarize-seg", "embed"] satisfies Task[]) {
      expect(roleFor(task)).toBeNull();
    }
  });
});

describe("the order cards are shown in", () => {
  /** The one group `byTask` makes when every model has the same task. */
  const firstGroup = (models: CatalogueModel[]) => {
    const groups = byTask(models);
    expect(groups).toHaveLength(1);
    return groups[0]!;
  };

  const speech = (id: string, size: number, accuracy?: number): CatalogueModel =>
    model({
      id,
      size_bytes: size,
      ...(accuracy === undefined ? {} : { accuracy: [{ lang: "en", accuracy }] }),
    });

  /**
   * Size alone put the worst model in the catalogue at the top of the speech section.
   * `zipformer-en` is 70 MB and measures 40 % accurate, so a reader scanning left to right met it
   * before anything that works.
   */
  it("puts the better model before the smaller one", () => {
    const group = firstGroup([
      speech("small-and-bad", 70, 0.4),
      speech("bigger-and-good", 160, 0.9),
    ]);
    expect(group.models.map((each: CatalogueModel) => each.id)).toEqual([
      "bigger-and-good",
      "small-and-bad",
    ]);
  });

  /**
   * After measured, not before it. Ahead of everything would let an unknown outrank a known-good
   * model, which is the mistake the accuracy cell exists to prevent; ahead of nothing would bury a
   * model nobody has got to yet.
   */
  it("sorts an unmeasured model after the measured ones and before nothing", () => {
    const group = firstGroup([
      speech("unmeasured", 10),
      speech("good", 500, 0.9),
      speech("bad", 20, 0.4),
    ]);
    expect(group.models.map((each: CatalogueModel) => each.id)).toEqual([
      "good",
      "bad",
      "unmeasured",
    ]);
  });

  it("still puts what you have installed first", () => {
    const mine = { ...speech("installed-and-bad", 900, 0.3), installed: true };
    const group = firstGroup([speech("good", 100, 0.95), mine]);
    expect(group.models[0]?.id).toBe("installed-and-bad");
  });
});

describe("models a newer one replaced", () => {
  const model = (id: string, extra: Partial<CatalogueModel> = {}): CatalogueModel => ({
    id,
    name: id,
    task: "asr",
    mode: "live",
    langs: ["vi"],
    license: "MIT",
    redistributable: true,
    gated: false,
    size_bytes: 1,
    installed: false,
    fits: true,
    min_ram_mb: 1,
    runnable: true,
    ...extra,
  });

  /**
   * Two cards for one model, and the only thing telling them apart was a parenthesis inside a
   * name. Reported as "sao lại có 2 card, có cái mới bỏ cái cũ hoặc có select version gì chứ?".
   */
  it("shows the replacement and not the model it replaced", () => {
    const shown = current([
      model("gipformer-65m", { superseded_by: "gipformer-1.5-68m" }),
      model("gipformer-1.5-68m"),
    ]);
    expect(shown.map((each: CatalogueModel) => each.id)).toEqual(["gipformer-1.5-68m"]);
  });

  /**
   * The exception that matters more than the rule. Hiding a superseded model that is on disk would
   * hide the only control that removes it — and the replacement is another download, so somebody
   * may be keeping the old one on purpose.
   */
  it("keeps one that is already installed", () => {
    const shown = current([
      model("gipformer-65m", { superseded_by: "gipformer-1.5-68m", installed: true }),
      model("gipformer-1.5-68m"),
    ]);
    expect(shown.map((each: CatalogueModel) => each.id)).toEqual([
      "gipformer-65m",
      "gipformer-1.5-68m",
    ]);
  });

  /** A promise the registry has not kept yet must not take a model away and put nothing back. */
  it("keeps one whose replacement is not published", () => {
    const shown = current([model("gipformer-65m", { superseded_by: "gipformer-2" })]);
    expect(shown.map((each: CatalogueModel) => each.id)).toEqual(["gipformer-65m"]);
  });
});
