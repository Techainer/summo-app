/**
 * From what a person selected on screen back to the bytes it came from.
 *
 * The draft panel lets a user drag across a phrase and ask for that phrase to be rewritten. The
 * daemon splices: it finds the selection in the section **verbatim** and replaces exactly those
 * bytes, so everything outside is byte-identical afterwards (`summo_engine::draft::refine`). That
 * only works while the text on screen is the text in the file — which is why the panel drew raw
 * Markdown, and why a reader saw `- [ ] @Ngọc Chốt spec API` in their own summary.
 *
 * So the panel renders, and this translates. It walks the source once, emitting the characters a
 * reader can actually see and remembering where each one came from; a selection is then found in
 * that visible text and cut back out of the *source*. The model is asked to revise the Markdown,
 * which is also the right thing to hand it: a phrase inside a list item should come back as a list
 * item.
 *
 * Deliberately not a Markdown parser. It only has to be right enough to find a run of characters,
 * and [`sourceOf`] returns `null` when it is not, at which point the caller can send what the user
 * selected and get the daemon's existing "that passage has moved on" answer.
 */

export interface Visible {
  /** What a reader sees, near enough. */
  text: string;
  /** For each character of `text`, the index in the source it came from. */
  map: number[];
}

/** A list marker, a heading marker, a quote — everything that is punctuation for the renderer. */
const LINE_PREFIX = /^(\s*)(?:[-*] \[[ xX]\] |[-*] |\d+\. |#{1,6} |> )/;

/**
 * The characters of `markdown` a reader can see, and where each one is.
 *
 * Whitespace is kept as it is, including newlines, so a selection that runs across two lines still
 * matches. Comments, emphasis markers, list bullets and link targets are dropped, because none of
 * them is on the screen.
 */
export function visible(markdown: string): Visible {
  const text: string[] = [];
  const map: number[] = [];
  const push = (from: number, count = 1) => {
    for (let n = 0; n < count; n += 1) {
      text.push(markdown[from + n] ?? "");
      map.push(from + n);
    }
  };

  let at = 0;
  let lineStart = true;
  while (at < markdown.length) {
    if (lineStart) {
      const prefix = LINE_PREFIX.exec(markdown.slice(at));
      if (prefix) {
        // The indentation is kept and the marker is not: indentation is space a reader sees, and a
        // bullet is drawn by the list rather than written in the text.
        push(at, prefix[1]?.length ?? 0);
        at += prefix[0].length;
      }
      lineStart = false;
    }

    const rest = markdown.slice(at);

    // The machine-readable state the vault carries in comments. Invisible, always.
    if (rest.startsWith("<!--")) {
      const end = markdown.indexOf("-->", at);
      at = end === -1 ? markdown.length : end + 3;
      continue;
    }

    // A link or an image: the label is visible, the target is not.
    const link = /^!?\[([^\]]*)\]\([^)\s]*\)/.exec(rest);
    if (link) {
      const label = link[1] ?? "";
      // The label starts after `[`, or after `![`.
      const from = at + (rest.startsWith("!") ? 2 : 1);
      push(from, label.length);
      at += link[0].length;
      continue;
    }

    // Emphasis, code and strike: markers around text that is otherwise itself.
    const marker = /^(\*\*|~~|\*|`)/.exec(rest);
    if (marker) {
      at += marker[0].length;
      continue;
    }

    if (markdown[at] === "\n") lineStart = true;
    push(at);
    at += 1;
  }

  return { text: text.join(""), map };
}

/**
 * The source that produced a run of visible text, or `null` when it cannot be located.
 *
 * `null` covers three honest cases: the selection is not in this section, it spans a boundary the
 * renderer collapses differently from this walk, or it appears more than once and there is no way
 * to know which one was meant. Guessing at any of them would rewrite the wrong bytes.
 */
export function sourceOf(markdown: string, selected: string): string | null {
  const wanted = selected.trim();
  if (wanted === "") return null;

  const seen = visible(markdown);
  const at = seen.text.indexOf(wanted);
  if (at === -1) return null;
  if (seen.text.indexOf(wanted, at + 1) !== -1) return null;

  const from = seen.map[at];
  const to = seen.map[at + wanted.length - 1];
  if (from === undefined || to === undefined) return null;

  // Emphasis the selection started or ended inside comes with it. The daemon replaces exactly these
  // bytes and leaves the rest alone, so cutting between a `**` and the words it wraps would leave
  // half a pair behind in the file — and `Chốt **` followed by whatever the model writes is a
  // section that renders as nonsense from then on.
  const before = at === 0 ? 0 : (seen.map[at - 1] ?? -1) + 1;
  const after = seen.map[at + wanted.length] ?? markdown.length;
  const start = MARKERS.test(markdown.slice(before, from)) ? before : from;
  const end = MARKERS.test(markdown.slice(to + 1, after)) ? after : to + 1;
  return markdown.slice(start, end);
}

/** What emphasis is written with. A list bullet is not here: it is not part of the phrase. */
const MARKERS = /^[*~`]+$/;
