import { describe, expect, it } from "vitest";

import { translator } from "../i18n";
import { describeError, explain, failureFrom, messageOf, same } from "./errors";

const t = translator("en", {
  "errors.import.no_audio": "That file has no audio in it.",
});

describe("explain", () => {
  it("uses the translation when the interface knows the code", () => {
    expect(explain({ error: "không có âm thanh", code: "import.no_audio" }, t)).toBe(
      "That file has no audio in it.",
    );
  });

  // Adding a code on the Rust side must never break a client that is a version behind.
  it("falls back to the daemon's own text for a code it has never seen", () => {
    expect(explain({ error: "không có âm thanh", code: "brand.new.code" }, t)).toBe(
      "không có âm thanh",
    );
  });

  // A checksum mismatch is a fault, not a message: the detail is the whole content.
  it("shows an uncoded failure verbatim", () => {
    expect(explain({ error: "checksum mismatch for model.onnx" }, t)).toBe(
      "checksum mismatch for model.onnx",
    );
  });

  // `t` renders a missing key as the key, which on screen would read `errors.import.no_audio`.
  it("never renders a bare key", () => {
    const shown = explain({ error: "real text", code: "not.in.the.catalog" }, t);
    expect(shown).not.toContain("errors.");
  });
});

describe("failureFrom", () => {
  it("reads the daemon's shape", () => {
    expect(failureFrom({ error: "nope", code: "note.no_title" })).toEqual({
      error: "nope",
      code: "note.no_title",
    });
  });

  it("keeps the text when there is no code", () => {
    expect(failureFrom({ error: "nope" })).toEqual({ error: "nope" });
  });

  it("ignores an empty code rather than carrying one that matches nothing", () => {
    expect(failureFrom({ error: "nope", code: "" })).toEqual({ error: "nope" });
  });

  // An HTML error page from a proxy, an empty 502. "[object Object]" on screen helps nobody.
  it("says the status when the body is not an error object at all", () => {
    expect(failureFrom("<html>502</html>", 502).error).toBe("HTTP 502");
    expect(failureFrom(null, 500).error).toBe("HTTP 500");
    expect(failureFrom({}, 404).error).toBe("HTTP 404");
  });

  it("has something to say even with no status", () => {
    expect(failureFrom(undefined).error).toBe("unknown error");
  });
});

describe("messageOf", () => {
  it("unwraps an Error", () => {
    expect(messageOf(new Error("boom"))).toBe("boom");
  });

  it("passes a thrown string through", () => {
    expect(messageOf("boom")).toBe("boom");
  });

  // A rejected promise with no reason must not render as "undefined".
  it("never renders undefined", () => {
    expect(messageOf(undefined)).toBe("unknown error");
    expect(messageOf(null)).toBe("unknown error");
    expect(messageOf("")).toBe("unknown error");
  });
});

// The recording session holds a `Failure` rather than throwing one, and handed it to `describeError`
// — which understood `Error` and `string` and nothing else, so every session failure rendered as
// `unknown error` on screen with the real reason one field away.
describe("a failure that was never thrown", () => {
  const t = {
    t: (key: string) =>
      ({
        "errors.session_refused": "Không bắt đầu ghi được.",
        "errors.unknown": "Có gì đó hỏng mà app không đọc được lý do.",
      })[key] ?? key,
    has: (key: string) => ["errors.session_refused", "errors.unknown"].includes(key),
  } as unknown as Parameters<typeof describeError>[1];

  it("is read, not discarded", () => {
    expect(describeError({ error: "session needs a live model" }, t)).toBe(
      "session needs a live model",
    );
  });

  it("is translated when it carries a code", () => {
    expect(describeError({ code: "session_refused", error: "refused" }, t)).toBe(
      "Không bắt đầu ghi được.",
    );
  });

  // An object with no `error` string is not a failure at all — it is whatever else was thrown — and
  // the last-resort sentence is still a sentence in the reader's language.
  it("still says something readable for something with no message at all", () => {
    expect(describeError({}, t)).toBe("Có gì đó hỏng mà app không đọc được lý do.");
  });
});

describe("same", () => {
  // The banner and the status bar are fed by two paths that both carry one refusal: the call that
  // was refused, and the daemon announcing it. Pressing record with no model printed the identical
  // sentence at the top of the window and along the bottom of it.
  it("matches on the code when both carry one", () => {
    expect(
      same({ error: "a", code: "session.no_model" }, { error: "b", code: "session.no_model" }),
    ).toBe(true);
    expect(
      same({ error: "a", code: "session.no_model" }, { error: "a", code: "session.no_vad" }),
    ).toBe(false);
  });

  it("falls back to the text when a code is missing", () => {
    expect(same({ error: "refused" }, { error: "refused" })).toBe(true);
    expect(same({ error: "refused" }, { error: "something else" })).toBe(false);
  });

  // Nothing is not the same as nothing: with no banner on screen there is no duplicate to suppress,
  // and treating two absences as equal would hide the status bar's only message.
  it("is false when either side is absent", () => {
    expect(same(null, null)).toBe(false);
    expect(same({ error: "refused" }, null)).toBe(false);
  });
});
