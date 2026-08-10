import { describe, expect, it } from "vitest";

import {
  confidenceLabel,
  correctionSummary,
  nameOptions,
  speakingTime,
  type Correction,
  type Person,
  type UnknownVoice,
} from "./people";

function person(id: string, name: string): Person {
  return { id, name, samples: 4, confirmed: 1, centroids: 2 };
}

function voice(suggestions: { id: string; similarity: number }[]): UnknownVoice {
  return {
    label: "S2",
    utterances: 12,
    seconds: 240,
    suggestions: suggestions.map((s) => ({ ...s, name: s.id })),
  };
}

describe("speakingTime", () => {
  it("uses seconds for a short interjection", () => {
    expect(speakingTime(12)).toBe("12 giây");
    expect(speakingTime(59)).toBe("59 giây");
  });

  it("uses minutes for a normal contribution", () => {
    expect(speakingTime(240)).toBe("4 phút");
  });

  it("uses hours once minutes stop being readable", () => {
    expect(speakingTime(3600)).toBe("1 giờ");
    expect(speakingTime(5400)).toBe("1 giờ 30 phút");
  });

  it("does not say zero minutes", () => {
    expect(speakingTime(7200)).toBe("2 giờ");
  });
});

describe("confidenceLabel", () => {
  it("describes a match in words, not a cosine", () => {
    expect(confidenceLabel(0.9)).toBe("rất giống");
    expect(confidenceLabel(0.65)).toBe("khá giống");
    expect(confidenceLabel(0.45)).toBe("hơi giống");
  });

  it("puts the boundaries where the thresholds are", () => {
    // 0.62 is SAME_VOICE in summo-diar; a match at exactly that should not read as a weak one.
    expect(confidenceLabel(0.62)).toBe("khá giống");
    expect(confidenceLabel(0.619)).toBe("hơi giống");
  });
});

describe("correctionSummary", () => {
  const base: Correction = {
    person: person("binh", "Bình"),
    relabelled_here: 12,
    relabelled_elsewhere: [],
    corrected_profiles: [],
  };

  it("says nothing when only the current meeting changed", () => {
    // Naming a voice obviously names the voice. Announcing it is noise.
    expect(correctionSummary(base)).toBeNull();
  });

  it("reports past meetings, because rewriting them unannounced is alarming", () => {
    const summary = correctionSummary({
      ...base,
      relabelled_elsewhere: [
        { meeting: "01A", utterances: 8 },
        { meeting: "01B", utterances: 3 },
      ],
    });
    expect(summary).toContain("11 câu");
    expect(summary).toContain("2 buổi họp");
  });

  it("reports profiles the correction took samples away from", () => {
    const summary = correctionSummary({ ...base, corrected_profiles: ["ngoc"] });
    expect(summary).toContain("gỡ nhầm khỏi 1 người");
  });

  it("reports both at once", () => {
    const summary = correctionSummary({
      ...base,
      relabelled_elsewhere: [{ meeting: "01A", utterances: 5 }],
      corrected_profiles: ["ngoc"],
    });
    expect(summary).toContain("5 câu");
    expect(summary).toContain("gỡ nhầm");
  });
});

describe("nameOptions", () => {
  const people = [person("ngoc", "Ngọc"), person("binh", "Bình"), person("an", "An")];

  it("puts the model's guesses first, in its order", () => {
    const options = nameOptions(
      voice([
        { id: "binh", similarity: 0.8 },
        { id: "ngoc", similarity: 0.6 },
      ]),
      people,
    );
    expect(options.map((p) => p.id)).toEqual(["binh", "ngoc", "an"]);
  });

  it("still offers everyone else, because the right answer is often unrecognised", () => {
    const options = nameOptions(voice([]), people);
    expect(options).toHaveLength(3);
  });

  it("sorts the rest by Vietnamese collation, not code points", () => {
    const options = nameOptions(voice([]), people);
    expect(options.map((p) => p.name)).toEqual(["An", "Bình", "Ngọc"]);
  });

  it("lists nobody twice", () => {
    const options = nameOptions(voice([{ id: "ngoc", similarity: 0.9 }]), people);
    expect(new Set(options.map((p) => p.id)).size).toBe(options.length);
  });

  it("ignores a suggestion for somebody who has since been deleted", () => {
    const options = nameOptions(voice([{ id: "gone", similarity: 0.9 }]), people);
    expect(options.map((p) => p.id)).toEqual(["an", "binh", "ngoc"]);
  });
});
